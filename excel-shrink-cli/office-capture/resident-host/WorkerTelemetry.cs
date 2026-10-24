using System.Text.Json.Nodes;

// Protocol observations, not guesses about Excel's internal activity. Callers
// hold ClientState.Gate; heartbeat and empty job polls never count as progress.
internal sealed class WorkerTelemetry
{
    private const int HistoryLimit = 24;
    private readonly Queue<JsonObject> events = new();
    private string state = "starting";
    private string? waitReason;
    private DateTimeOffset stateSince = DateTimeOffset.UtcNow;
    private DateTimeOffset? lastProgress;
    private JsonObject? lastEvent, lastSuccess, activeJob;
    private long sequence, uploadedImages, completedCommands;

    public void Transition(string next, string? waitingFor, string message)
    {
        if (state == next && waitReason == waitingFor) return;
        state = next;
        waitReason = waitingFor;
        stateSince = DateTimeOffset.UtcNow;
        Record("state_changed", message);
    }

    public void Configured()
    {
        Transition("waiting_for_job", "coordinator", "Add-in connected and configured.");
        Progress("addin_ready", "Add-in configuration completed.");
    }

    public void Started(JsonObject command)
    {
        activeJob = Describe(command);
        var kind = (string?)command["kind"];
        Transition(kind switch {
            "plan" => "planning_sheet",
            "images" => "providing_images",
            "release" => "releasing",
            _ => "capturing"
        }, null, $"Office.js received {kind} command.");
        Record("job_dispatched", "Command delivered to the add-in.", activeJob);
    }

    public void ImageReceived(JsonNode tile)
    {
        uploadedImages++;
        var details = Pick(tile, "sheet_index", "sheet_name", "tile_index", "range");
        details["command_id"] = activeJob?["id"]?.DeepClone();
        Progress("image_received", "Capture image received and validated.", details);
    }

    public void Finished(bool ok, JsonObject? error)
    {
        if (ok) {
            completedCommands++;
            Progress("job_completed", "Office.js command completed successfully.", activeJob);
            Transition("waiting_for_job", "coordinator", "Waiting for the next coordinator command.");
        } else {
            Transition((string?)error?["code"] == "InvalidOperationInCellEditMode" ? "blocked_edit_mode" : "client_error",
                null, (string?)error?["message"] ?? "Office.js reported a failed command.");
            Record("job_failed", (string?)error?["message"] ?? "Command failed.", activeJob);
        }
        activeJob = null;
    }

    public void Restarted()
    {
        Transition("client_error", null, "The task pane restarted after a command was dispatched.");
    }

    private void Progress(string kind, string message, JsonObject? details = null)
    {
        lastProgress = DateTimeOffset.UtcNow;
        lastSuccess = Record(kind, message, details);
    }

    private JsonObject Record(string kind, string message, JsonObject? details = null)
    {
        var entry = new JsonObject {
            ["sequence"] = ++sequence, ["at"] = DateTimeOffset.UtcNow.ToString("O"),
            ["kind"] = kind, ["message"] = message
        };
        if (details is not null) entry["details"] = details.DeepClone();
        events.Enqueue(entry);
        while (events.Count > HistoryLimit) events.Dequeue();
        lastEvent = entry;
        return entry;
    }

    public JsonObject Snapshot() => new() {
        ["state"] = state, ["state_since"] = stateSince.ToString("O"), ["wait_reason"] = waitReason,
        ["last_progress_at"] = lastProgress?.ToString("O"), ["last_event"] = lastEvent?.DeepClone(),
        ["last_success"] = lastSuccess?.DeepClone(), ["active_job"] = activeJob?.DeepClone(),
        ["events"] = new JsonArray(events.Select(entry => (JsonNode)entry.DeepClone()).ToArray()),
        ["uploaded_images"] = uploadedImages, ["completed_commands"] = completedCommands
    };

    private static JsonObject Describe(JsonObject command)
    {
        var result = Pick(command, "id", "kind", "sheet_index", "sheet_name");
        var tiles = (command["tiles"] as JsonArray)?.OfType<JsonObject>().ToArray() ?? [];
        var sheets = (command["sheets"] as JsonArray)?.OfType<JsonObject>().ToArray() ?? [];
        result["tile_count"] = tiles.Length;
        result["sheet_count"] = tiles.Length > 0 ? tiles.Select(tile => tile["sheet_index"]?.ToJsonString()).Distinct().Count() : sheets.Length;
        result["locations"] = new JsonArray((tiles.Length > 0 ? tiles : sheets).Take(8)
            .Select(item => (JsonNode)Pick(item, "sheet_index", "sheet_name", "tile_index", "range", "name", "index")).ToArray());
        return result;
    }

    private static JsonObject Pick(JsonNode source, params string[] names)
    {
        var result = new JsonObject();
        foreach (var name in names) if (source[name] is JsonNode value) result[name] = value.DeepClone();
        return result;
    }
}
