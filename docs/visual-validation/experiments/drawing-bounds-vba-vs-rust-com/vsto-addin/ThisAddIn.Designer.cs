#pragma warning disable 414
namespace DrawingBoundsVsto
{
    [Microsoft.VisualStudio.Tools.Applications.Runtime.StartupObjectAttribute(0)]
    [global::System.Security.Permissions.PermissionSetAttribute(global::System.Security.Permissions.SecurityAction.Demand, Name="FullTrust")]
    public sealed partial class ThisAddIn : Microsoft.Office.Tools.AddInBase
    {
        internal Microsoft.Office.Tools.CustomTaskPaneCollection CustomTaskPanes;
        internal Microsoft.Office.Tools.SmartTagCollection VstoSmartTags;
        private global::System.Object missing = global::System.Type.Missing;
        internal Microsoft.Office.Interop.Excel.Application Application;

        public ThisAddIn(global::Microsoft.Office.Tools.Excel.ApplicationFactory factory, global::System.IServiceProvider serviceProvider)
            : base(factory, serviceProvider, "AddIn", "ThisAddIn")
        {
            Globals.Factory = factory;
        }

        protected override void Initialize()
        {
            base.Initialize();
            this.Application = this.GetHostItem<Microsoft.Office.Interop.Excel.Application>(typeof(Microsoft.Office.Interop.Excel.Application), "Application");
            Globals.ThisAddIn = this;
            global::System.Windows.Forms.Application.EnableVisualStyles();
            this.InitializeCachedData();
            this.InitializeControls();
            this.InitializeComponents();
            this.InitializeData();
        }

        protected override void FinishInitialization()
        {
            this.InternalStartup();
            this.OnStartup();
        }

        protected override void InitializeDataBindings()
        {
            this.BeginInitialization();
            this.BindToData();
            this.EndInitialization();
        }

        private void InitializeCachedData()
        {
            if (this.DataHost == null) return;
            if (this.DataHost.IsCacheInitialized) this.DataHost.FillCachedData(this);
        }

        private void InitializeData() { }
        private void BindToData() { }
        private void InitializeComponents() { }

        private void BeginInitialization()
        {
            this.BeginInit();
            this.CustomTaskPanes.BeginInit();
            this.VstoSmartTags.BeginInit();
        }

        private void EndInitialization()
        {
            this.VstoSmartTags.EndInit();
            this.CustomTaskPanes.EndInit();
            this.EndInit();
        }

        private void InitializeControls()
        {
            this.CustomTaskPanes = Globals.Factory.CreateCustomTaskPaneCollection(null, null, "CustomTaskPanes", "CustomTaskPanes", this);
            this.VstoSmartTags = Globals.Factory.CreateSmartTagCollection(null, null, "VstoSmartTags", "VstoSmartTags", this);
        }

        protected override void OnShutdown()
        {
            this.VstoSmartTags.Dispose();
            this.CustomTaskPanes.Dispose();
            base.OnShutdown();
        }
    }

    internal sealed partial class Globals
    {
        private Globals() { }
        private static ThisAddIn _ThisAddIn;
        private static global::Microsoft.Office.Tools.Excel.ApplicationFactory _factory;
        private static ThisRibbonCollection _ThisRibbonCollection;

        internal static ThisAddIn ThisAddIn
        {
            get { return _ThisAddIn; }
            set
            {
                if (_ThisAddIn == null) _ThisAddIn = value;
                else throw new System.NotSupportedException();
            }
        }

        internal static global::Microsoft.Office.Tools.Excel.ApplicationFactory Factory
        {
            get { return _factory; }
            set
            {
                if (_factory == null) _factory = value;
                else throw new System.NotSupportedException();
            }
        }

        internal static ThisRibbonCollection Ribbons
        {
            get
            {
                if (_ThisRibbonCollection == null)
                    _ThisRibbonCollection = new ThisRibbonCollection(_factory.GetRibbonFactory());
                return _ThisRibbonCollection;
            }
        }
    }

    internal sealed partial class ThisRibbonCollection : Microsoft.Office.Tools.Ribbon.RibbonCollectionBase
    {
        internal ThisRibbonCollection(global::Microsoft.Office.Tools.Ribbon.RibbonFactory factory) : base(factory) { }
    }
}
