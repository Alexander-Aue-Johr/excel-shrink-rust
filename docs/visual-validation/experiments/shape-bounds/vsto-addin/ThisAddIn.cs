using System;

namespace DrawingBoundsVsto
{
    public partial class ThisAddIn
    {
        private DrawingBoundsAutomation automation;

        protected override object RequestComAddInAutomationService()
        {
            if (automation == null)
            {
                automation = new DrawingBoundsAutomation(this.Application);
            }

            return automation;
        }

        private void ThisAddIn_Startup(object sender, EventArgs e)
        {
        }

        private void ThisAddIn_Shutdown(object sender, EventArgs e)
        {
        }

        private void InternalStartup()
        {
            this.Startup += new EventHandler(ThisAddIn_Startup);
            this.Shutdown += new EventHandler(ThisAddIn_Shutdown);
        }
    }
}
