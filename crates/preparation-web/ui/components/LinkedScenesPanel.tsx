import { useEffect, useRef, useState, type ReactNode } from "react";
import { Expand, Minimize2 } from "lucide-react";
import SheetConnections from "./SheetConnections";

interface Props {
  title: string;
  sheetId: string | null;
  worker: ReactNode;
  atlas?: ReactNode;
  controls?: ReactNode;
  footer?: ReactNode;
}

/** Keep linked diagrams, connection paths and controls in one resizable panel. */
export default function LinkedScenesPanel({
  title,
  sheetId,
  worker,
  atlas,
  controls,
  footer,
}: Props) {
  const [expanded, setExpanded] = useState(false);
  const savedScrollY = useRef(0);
  useEffect(() => {
    if (!expanded) return;
    const overflow = document.body.style.overflow;
    const scrollY = savedScrollY.current;
    document.body.style.overflow = "hidden";
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setExpanded(false);
    };
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("keydown", escape);
      document.body.style.overflow = overflow;
      window.scrollTo(0, scrollY);
    };
  }, [expanded]);

  return (
    <section
      className={`panel linked-scenes${expanded ? " is-expanded" : ""}`}
      aria-label="Linked processing diagrams"
    >
      <SheetConnections sheetId={sheetId} />
      <header className="panel-header linked-header">
        <h2>{title}</h2>
        <div className="linked-header-controls">
          {controls}
          <button
            className="icon-button linked-expand"
            aria-label={
              expanded ? "Restore linked diagrams" : "Maximize linked diagrams"
            }
            title={expanded ? "Restore panel (Esc)" : "Maximize both diagrams"}
            aria-pressed={expanded}
            onClick={() => {
              if (!expanded) savedScrollY.current = window.scrollY;
              setExpanded(!expanded);
            }}
          >
            {expanded ? <Minimize2 size={16} /> : <Expand size={16} />}
          </button>
        </div>
      </header>
      <div className={`linked-views${atlas ? " has-atlas" : ""}`}>
        {worker}
        {atlas}
      </div>
      <footer className="linked-footer">{footer}</footer>
    </section>
  );
}
