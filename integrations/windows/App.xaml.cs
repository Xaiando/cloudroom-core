using System.IO;
using System.Windows;

namespace Cloudroom.Desktop;

public partial class App : Application
{
    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        string config = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "CloudroomCore", "core.json");
        string? smoke = null;
        bool explicitConfig = false;
        for (int i = 0; i < e.Args.Length; i++)
        {
            if (e.Args[i] == "--config" && i + 1 < e.Args.Length) { config = e.Args[++i]; explicitConfig = true; }
            else if (e.Args[i] == "--smoke-test" && i + 1 < e.Args.Length) smoke = e.Args[++i];
        }
        if (smoke != null) System.Windows.Media.RenderOptions.ProcessRenderMode = System.Windows.Interop.RenderMode.SoftwareOnly;
        var window = new MainWindow(config, smoke, discoverSettings: !explicitConfig);
        MainWindow = window;
        window.Show();
    }
}
