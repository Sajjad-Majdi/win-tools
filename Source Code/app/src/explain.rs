//! "What is this process?": instant offline answer from a built-in list of common Windows processes, plus checks
//! that matter (signature, location, look-alike names). Unknown processes can be asked to Gemini (ai.rs).
use crate::procs::Proc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// closing it loses nothing important
    Safe,
    /// it is an app the user works in: unsaved work may be lost
    Work,
    /// Windows needs it; ending it can freeze or restart the PC
    Critical,
    /// looks wrong (unsigned in a temp folder, look-alike name, ...)
    Suspicious,
    Unknown,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Safe => "safe_to_end",
            Verdict::Work => "ends_your_work",
            Verdict::Critical => "system_critical",
            Verdict::Suspicious => "suspicious",
            Verdict::Unknown => "unknown",
        }
    }
    pub fn label_en(self) -> &'static str {
        match self {
            Verdict::Safe => "Safe to end",
            Verdict::Work => "Ends your work",
            Verdict::Critical => "System critical",
            Verdict::Suspicious => "Suspicious",
            Verdict::Unknown => "Unknown",
        }
    }
    pub fn label_fa(self) -> &'static str {
        match self {
            Verdict::Safe => "بستنش بی‌خطر است",
            Verdict::Work => "کار باز از دست می‌رود",
            Verdict::Critical => "حیاتی برای ویندوز",
            Verdict::Suspicious => "مشکوک",
            Verdict::Unknown => "ناشناخته",
        }
    }
    fn parse(s: &str) -> Verdict {
        match s {
            "s" => Verdict::Safe,
            "w" => Verdict::Work,
            "c" => Verdict::Critical,
            _ => Verdict::Unknown,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Answer {
    pub verdict: Verdict,
    pub what_en: String,
    pub what_fa: String,
    pub maker: String,
    pub notes: Vec<String>,
    /// "builtin", "file-info" or "ai"
    pub source: &'static str,
    /// what stops if it is ended (e.g. "the VPN / proxy connection drops"); empty = use the verdict's sentence
    pub ends_en: String,
}

// name | maker | verdict (s/w/c) | English | Persian
const KB: &str = "\
system|Microsoft|c|The Windows kernel. Runs drivers and core system work.|هستهٔ ویندوز. درایورها و کارهای اصلی سیستم را اجرا می‌کند.
system idle process|Microsoft|c|Not a real program: shows how much of the CPU is free.|برنامهٔ واقعی نیست؛ مقدار آزاد پردازنده را نشان می‌دهد.
registry|Microsoft|c|Holds the Windows registry in memory.|رجیستری ویندوز را در حافظه نگه می‌دارد.
memory compression|Microsoft|c|Compresses unused memory so more fits in RAM.|حافظهٔ بی‌کار را فشرده می‌کند تا رم بیشتری آزاد بماند.
secure system|Microsoft|c|Virtualization-based security of Windows.|بخش امنیتی مجازی‌سازی‌شدهٔ ویندوز.
smss.exe|Microsoft|c|Session Manager: starts the Windows user sessions.|مدیر نشست‌ها؛ نشست‌های ویندوز را راه می‌اندازد.
csrss.exe|Microsoft|c|Client/Server Runtime: core part of the Windows user session. Ending it crashes Windows.|بخش اصلی نشست کاربر. بستنش ویندوز را از کار می‌اندازد.
wininit.exe|Microsoft|c|Starts the core Windows services at boot.|سرویس‌های اصلی ویندوز را هنگام روشن‌شدن اجرا می‌کند.
winlogon.exe|Microsoft|c|Handles sign-in, lock screen and Ctrl+Alt+Del.|ورود، صفحهٔ قفل و Ctrl+Alt+Del را مدیریت می‌کند.
services.exe|Microsoft|c|Service Control Manager: starts and stops all Windows services.|مدیر سرویس‌ها؛ همهٔ سرویس‌های ویندوز را اجرا و متوقف می‌کند.
lsass.exe|Microsoft|c|Local Security Authority: passwords, sign-in and security tokens.|احراز هویت، رمزها و توکن‌های امنیتی ویندوز.
lsaiso.exe|Microsoft|c|Isolated part of the security authority (Credential Guard).|بخش ایزولهٔ احراز هویت (Credential Guard).
svchost.exe|Microsoft|c|Host for Windows services. Each copy runs one or more services; see its services before ending it.|میزبان سرویس‌های ویندوز. هر نمونه یک یا چند سرویس را اجرا می‌کند.
dwm.exe|Microsoft|c|Desktop Window Manager: draws every window on screen. Windows restarts it if it ends.|همهٔ پنجره‌ها را روی صفحه رسم می‌کند. اگر بسته شود، ویندوز دوباره اجرایش می‌کند.
fontdrvhost.exe|Microsoft|c|Loads fonts in a safe sandbox.|فونت‌ها را در محیطی امن بارگذاری می‌کند.
explorer.exe|Microsoft|w|The taskbar, Start menu and File Explorer. Ending it hides the taskbar until it restarts.|نوار وظیفه، منوی استارت و فایل‌اکسپلورر. با بستنش نوار وظیفه تا اجرای دوباره پنهان می‌شود.
sihost.exe|Microsoft|c|Shell Infrastructure Host: Start menu, notifications and other shell parts.|زیرساخت پوسته؛ منوی استارت، اعلان‌ها و بخش‌های دیگر.
ctfmon.exe|Microsoft|s|Keyboard layouts, input methods and the touch keyboard.|چیدمان صفحه‌کلید و روش‌های ورود متن.
taskhostw.exe|Microsoft|s|Host for scheduled tasks and background Windows tasks.|میزبان کارهای زمان‌بندی‌شده و پس‌زمینهٔ ویندوز.
runtimebroker.exe|Microsoft|s|Checks permissions for Store apps (camera, location, ...).|دسترسی‌های برنامه‌های استور را بررسی می‌کند.
searchhost.exe|Microsoft|s|Windows Search box in the taskbar.|جعبهٔ جست‌وجوی ویندوز در نوار وظیفه.
searchindexer.exe|Microsoft|s|Builds the Windows Search index of your files.|فهرست جست‌وجوی فایل‌ها را می‌سازد.
searchprotocolhost.exe|Microsoft|s|Reads files for the Windows Search index.|فایل‌ها را برای فهرست جست‌وجو می‌خواند.
searchfilterhost.exe|Microsoft|s|Extracts text from files for Windows Search.|متن فایل‌ها را برای جست‌وجوی ویندوز استخراج می‌کند.
startmenuexperiencehost.exe|Microsoft|s|The Start menu. Windows restarts it when needed.|منوی استارت؛ ویندوز در صورت نیاز دوباره اجرایش می‌کند.
shellexperiencehost.exe|Microsoft|s|Taskbar flyouts: clock, volume, network, notifications.|پنجره‌های کوچک نوار وظیفه: ساعت، صدا، شبکه، اعلان‌ها.
textinputhost.exe|Microsoft|s|Emoji panel, clipboard history and touch keyboard.|پنل ایموجی، تاریخچهٔ کلیپ‌بورد و صفحه‌کلید لمسی.
applicationframehost.exe|Microsoft|w|Window frame for Store apps (Settings, Calculator, ...).|قاب پنجرهٔ برنامه‌های استور مثل تنظیمات و ماشین‌حساب.
systemsettings.exe|Microsoft|s|The Windows Settings app.|برنامهٔ تنظیمات ویندوز.
msmpeng.exe|Microsoft|c|Microsoft Defender antivirus engine. Protects the PC; Windows will not let you end it.|موتور آنتی‌ویروس مایکروسافت دیفندر. از سیستم محافظت می‌کند و بسته نمی‌شود.
mpdefendercoreservice.exe|Microsoft|c|Microsoft Defender core service.|سرویس اصلی مایکروسافت دیفندر.
nissrv.exe|Microsoft|c|Defender network inspection.|بازرسی شبکهٔ دیفندر.
securityhealthservice.exe|Microsoft|s|Windows Security health service.|سرویس سلامت امنیت ویندوز.
securityhealthsystray.exe|Microsoft|s|Windows Security icon in the tray.|آیکن امنیت ویندوز در سینی سیستم.
spoolsv.exe|Microsoft|s|Print spooler. Ending it stops printing until restart.|صف چاپ. با بستنش تا اجرای دوباره چاپ نمی‌شود.
audiodg.exe|Microsoft|s|Windows audio engine. Sound stops for a moment if ended.|موتور صدای ویندوز. با بستنش صدا لحظه‌ای قطع می‌شود.
conhost.exe|Microsoft|s|Console window host for command-line programs.|میزبان پنجرهٔ کنسول برنامه‌های خط فرمان.
dllhost.exe|Microsoft|s|COM surrogate: runs helper code such as thumbnail makers.|میزبان COM؛ کدهای کمکی مثل ساخت تصویر بندانگشتی را اجرا می‌کند.
wmiprvse.exe|Microsoft|s|WMI provider host: answers system information queries.|به درخواست‌های اطلاعات سیستم (WMI) پاسخ می‌دهد.
wudfhost.exe|Microsoft|c|Host for user-mode drivers (USB, sensors, ...).|میزبان درایورهای حالت کاربر مثل USB و حسگرها.
lsm.exe|Microsoft|c|Local Session Manager.|مدیر نشست‌های محلی.
dashost.exe|Microsoft|s|Device association: pairs Bluetooth and network devices.|اتصال دستگاه‌ها مثل بلوتوث را مدیریت می‌کند.
backgroundtaskhost.exe|Microsoft|s|Runs background tasks of Store apps.|کارهای پس‌زمینهٔ برنامه‌های استور را اجرا می‌کند.
smartscreen.exe|Microsoft|s|SmartScreen: checks downloaded files and apps.|فایل‌ها و برنامه‌های دانلودشده را بررسی می‌کند.
compattelrunner.exe|Microsoft|s|Windows compatibility and telemetry checks.|بررسی سازگاری و ارسال داده‌های تله‌متری ویندوز.
tiworker.exe|Microsoft|s|Windows Update installer worker. Ending it can delay updates.|نصب‌کنندهٔ به‌روزرسانی ویندوز. بستنش به‌روزرسانی را عقب می‌اندازد.
trustedinstaller.exe|Microsoft|s|Windows Modules Installer (updates and features).|نصب‌کنندهٔ ماژول‌ها و به‌روزرسانی‌های ویندوز.
msedge.exe|Microsoft|w|Microsoft Edge browser. Ending it closes its tabs.|مرورگر اج. با بستنش تب‌ها بسته می‌شوند.
msedgewebview2.exe|Microsoft|w|Edge WebView2: shows web content inside other apps (Widgets, Teams, ...).|محتوای وب را درون برنامه‌های دیگر نشان می‌دهد.
widgets.exe|Microsoft|s|The Widgets board.|پنل ویجت‌ها.
phoneexperiencehost.exe|Microsoft|s|Phone Link app in the background.|برنامهٔ Phone Link در پس‌زمینه.
onedrive.exe|Microsoft|s|OneDrive sync. Ending it pauses syncing.|همگام‌سازی وان‌درایو. با بستنش همگام‌سازی متوقف می‌شود.
taskmgr.exe|Microsoft|s|The Windows Task Manager.|تسک منیجر ویندوز.
notepad.exe|Microsoft|w|Notepad. Unsaved text is lost.|دفترچه یادداشت. متن ذخیره‌نشده از دست می‌رود.
code.exe|Microsoft|w|Visual Studio Code editor. Unsaved files may be lost.|ویرایشگر VS Code. فایل‌های ذخیره‌نشده ممکن است از دست بروند.
chrome.exe|Google|w|Google Chrome. Each tab and extension is its own process; ending the main one closes the browser.|مرورگر کروم. هر تب یک پردازهٔ جداست؛ بستن پردازهٔ اصلی مرورگر را می‌بندد.
firefox.exe|Mozilla|w|Firefox browser.|مرورگر فایرفاکس.
telegram.exe|Telegram|w|Telegram Desktop.|تلگرام دسکتاپ.
idman.exe|Tonec|w|Internet Download Manager. Downloads pause if ended.|اینترنت دانلود منیجر. با بستنش دانلودها متوقف می‌شوند.
iemonitor.exe|Tonec|s|IDM browser integration helper.|کمک‌کنندهٔ اتصال IDM به مرورگر.
claude.exe|Anthropic|w|Claude desktop app.|برنامهٔ دسکتاپ Claude.
everything.exe|voidtools|s|Everything file search (keeps the file index).|جست‌وجوی فایل Everything؛ فهرست فایل‌ها را نگه می‌دارد.
v2rayn.exe|2dust|w|v2rayN proxy client. Ending it cuts the VPN/proxy.|کلاینت پراکسی v2rayN. با بستنش اتصال پراکسی قطع می‌شود.
xray.exe|XTLS|w|Xray proxy core used by v2rayN.|هستهٔ پراکسی Xray که v2rayN از آن استفاده می‌کند.
windscribe.exe|Windscribe|w|Windscribe VPN.|وی‌پی‌ان Windscribe.
nvcontainer.exe|NVIDIA|s|NVIDIA helper container (GeForce Experience, telemetry).|کمک‌کنندهٔ انویدیا (GeForce Experience و تله‌متری).
nvdisplay.container.exe|NVIDIA|s|NVIDIA display driver helper (control panel).|کمک‌کنندهٔ درایور گرافیک انویدیا.
radeonsoftware.exe|AMD|s|AMD Radeon Software.|نرم‌افزار گرافیک AMD.
amdrsserv.exe|AMD|s|AMD Radeon settings service.|سرویس تنظیمات گرافیک AMD.
rtkauduservice64.exe|Realtek|s|Realtek audio helper.|کمک‌کنندهٔ صدای Realtek.
python.exe|Python|w|A Python program. Check its command line to see which script it runs.|یک برنامهٔ پایتون؛ خط فرمانش نشان می‌دهد کدام اسکریپت است.
pythonw.exe|Python|w|A Python program without a console window.|برنامهٔ پایتون بدون پنجرهٔ کنسول.
node.exe|OpenJS|w|A Node.js program. Check its command line.|یک برنامهٔ Node.js؛ خط فرمانش را ببینید.
powershell.exe|Microsoft|w|Windows PowerShell. A script may be running in it.|پاورشل ویندوز؛ ممکن است اسکریپتی در آن اجرا باشد.
pwsh.exe|Microsoft|w|PowerShell 7.|پاورشل ۷.
cmd.exe|Microsoft|w|Command Prompt.|خط فرمان ویندوز.
windowsterminal.exe|Microsoft|w|Windows Terminal.|ترمینال ویندوز.
openconsole.exe|Microsoft|w|Console host of Windows Terminal.|میزبان کنسول ترمینال ویندوز.
bash.exe|Git / GNU|w|Bash shell (Git for Windows).|پوستهٔ Bash (Git for Windows).
git.exe|Git|w|Git command.|دستور گیت.
cargo.exe|Rust|w|Rust build tool.|ابزار ساخت Rust.
rusttaskmanager.exe|Rust Task Manager|s|This Task Manager.|همین تسک منیجر.
wintask.exe|Rust Task Manager|s|This Task Manager (before 0.2.0).|همین تسک منیجر (نسخهٔ قدیمی).
wtm.exe|Rust Task Manager|s|Rust Task Manager command line (for agents and scripts).|خط فرمان Rust Task Manager برای ایجنت‌ها و اسکریپت‌ها.
rustfileexplorer.exe|Rust File Explorer|w|Rust File Explorer, the file manager. Running copies stop if it is ended.|فایل منیجر Rust File Explorer. کپی‌های در حال انجام متوقف می‌شوند.
wfiles.exe|Rust File Explorer|w|Rust File Explorer (before 0.2.0). Running copies stop if it is ended.|فایل منیجر Rust File Explorer (نسخهٔ قدیمی). کپی‌های در حال انجام متوقف می‌شوند.
wfm.exe|Rust File Explorer|s|Rust File Explorer command line.|خط فرمان فایل منیجر Rust File Explorer.
lockapp.exe|Microsoft|s|The lock screen.|صفحهٔ قفل ویندوز.
searchapp.exe|Microsoft|s|Windows Search in the taskbar (older Windows 11 builds).|جست‌وجوی ویندوز در نوار وظیفه.
gamebar.exe|Microsoft|s|Xbox Game Bar overlay.|نوار بازی ایکس‌باکس.
gamebarftserver.exe|Microsoft|s|Xbox Game Bar helper.|کمک‌کنندهٔ نوار بازی ایکس‌باکس.
yourphone.exe|Microsoft|s|Phone Link app.|برنامهٔ Phone Link.
crossdeviceservice.exe|Microsoft|s|Connects this PC with your phone and other devices.|اتصال رایانه با گوشی و دستگاه‌های دیگر.
msedgeupdate.exe|Microsoft|s|Edge updater.|به‌روزرسان مرورگر اج.
microsoftedgeupdate.exe|Microsoft|s|Edge updater.|به‌روزرسان مرورگر اج.
officeclicktorun.exe|Microsoft|s|Office installer and updater service.|سرویس نصب و به‌روزرسانی آفیس.
winword.exe|Microsoft|w|Microsoft Word. Unsaved documents are lost.|مایکروسافت ورد. سندهای ذخیره‌نشده از دست می‌روند.
excel.exe|Microsoft|w|Microsoft Excel. Unsaved workbooks are lost.|مایکروسافت اکسل. فایل‌های ذخیره‌نشده از دست می‌روند.
powerpnt.exe|Microsoft|w|Microsoft PowerPoint. Unsaved slides are lost.|مایکروسافت پاورپوینت. اسلایدهای ذخیره‌نشده از دست می‌روند.
outlook.exe|Microsoft|w|Microsoft Outlook mail.|ایمیل آوت‌لوک.
teams.exe|Microsoft|w|Microsoft Teams.|مایکروسافت تیمز.
ms-teams.exe|Microsoft|w|Microsoft Teams.|مایکروسافت تیمز.
mspaint.exe|Microsoft|w|Paint. Unsaved pictures are lost.|برنامهٔ پینت. تصویرهای ذخیره‌نشده از دست می‌روند.
calculatorapp.exe|Microsoft|s|Calculator.|ماشین‌حساب.
snippingtool.exe|Microsoft|w|Snipping Tool.|ابزار اسکرین‌شات.
photos.exe|Microsoft|s|Photos app.|برنامهٔ عکس‌ها.
vlc.exe|VideoLAN|s|VLC media player.|پخش‌کنندهٔ VLC.
spotify.exe|Spotify|s|Spotify music player.|پخش‌کنندهٔ اسپاتیفای.
discord.exe|Discord|w|Discord chat.|برنامهٔ گفت‌وگوی دیسکورد.
steam.exe|Valve|w|Steam game client. Running downloads and games stop.|کلاینت بازی استیم. دانلودها و بازی‌ها متوقف می‌شوند.
steamwebhelper.exe|Valve|s|Steam's built-in browser.|مرورگر داخلی استیم.
zoom.exe|Zoom|w|Zoom meetings. A running meeting ends.|زوم. جلسهٔ در حال اجرا تمام می‌شود.
anydesk.exe|AnyDesk|w|AnyDesk remote desktop.|دسترسی از راه دور AnyDesk.
7zfm.exe|Igor Pavlov|w|7-Zip file manager.|مدیر فایل 7-Zip.
7zg.exe|Igor Pavlov|w|7-Zip working on an archive. Ending it stops the extract or compress.|سون‌زیپ در حال کار روی فایل فشرده.
winrar.exe|win.rar GmbH|w|WinRAR. Ending it stops the extract or compress.|وین‌رر. استخراج یا فشرده‌سازی متوقف می‌شود.
obs64.exe|OBS Project|w|OBS Studio. A running recording or stream stops.|اوبی‌اس. ضبط یا پخش زنده متوقف می‌شود.
nvidia web helper.exe|NVIDIA|s|NVIDIA app web helper.|کمک‌کنندهٔ وب انویدیا.
nvsphelper64.exe|NVIDIA|s|NVIDIA overlay / ShadowPlay helper.|کمک‌کنندهٔ اوورلی انویدیا.
nvidia share.exe|NVIDIA|s|NVIDIA overlay.|اوورلی انویدیا.
amdrsserv.exe|AMD|s|AMD Radeon settings service.|سرویس تنظیمات گرافیک AMD.
atieclxx.exe|AMD|s|AMD display driver helper.|کمک‌کنندهٔ درایور گرافیک AMD.
atiesrxx.exe|AMD|s|AMD display driver event service.|سرویس رویدادهای درایور AMD.
igfxem.exe|Intel|s|Intel graphics helper.|کمک‌کنندهٔ گرافیک اینتل.
armourycrate.service.exe|ASUS|s|ASUS Armoury Crate service (fans, lighting, profiles). Settings it applies may reset.|سرویس آرموری کریت ایسوس (فن، نور، پروفایل‌ها).
armourycrate.usersessionhelper.exe|ASUS|s|ASUS Armoury Crate helper.|کمک‌کنندهٔ آرموری کریت ایسوس.
asusoptimization.exe|ASUS|s|ASUS system optimization service.|سرویس بهینه‌سازی ایسوس.
lightingservice.exe|ASUS|s|ASUS Aura RGB lighting service.|سرویس نورپردازی ایسوس.
gamesdk.exe|ASUS|s|ASUS GameSDK service.|سرویس GameSDK ایسوس.
rogliveservice.exe|ASUS|s|ROG Live service (ASUS updates and news).|سرویس ROG Live ایسوس.
dropbox.exe|Dropbox|s|Dropbox sync. Sync pauses.|همگام‌سازی دراپ‌باکس.
googledrivefs.exe|Google|s|Google Drive for desktop. Sync pauses.|گوگل درایو دسکتاپ. همگام‌سازی متوقف می‌شود.
googleupdater.exe|Google|s|Google software updater.|به‌روزرسان نرم‌افزارهای گوگل.
updater.exe|-|s|A program's updater. Check its path to see whose it is.|به‌روزرسان یک برنامه؛ مسیرش نشان می‌دهد مال کیست.
java.exe|Oracle / OpenJDK|w|A Java program. Check its command line.|یک برنامهٔ جاوا؛ خط فرمانش را ببینید.
javaw.exe|Oracle / OpenJDK|w|A Java program without a console.|برنامهٔ جاوا بدون کنسول.
rustc.exe|Rust|w|Rust compiler (a build is running).|کامپایلر Rust (یک ساخت در حال اجراست).
llama-server.exe|llama.cpp|s|Local AI model server (local AI features).|سرور مدل هوش مصنوعی محلی.
ollama.exe|Ollama|s|Ollama local AI model server.|سرور مدل هوش مصنوعی Ollama.
docker desktop.exe|Docker|w|Docker Desktop. Running containers stop.|داکر دسکتاپ. کانتینرها متوقف می‌شوند.
vmmem|Microsoft|w|Memory of WSL or virtual machines. Ending it stops them.|حافظهٔ WSL یا ماشین‌های مجازی.
vmmemwsl|Microsoft|w|Memory of WSL (Linux on Windows). Ending it stops WSL.|حافظهٔ WSL. بستنش WSL را متوقف می‌کند.
wsl.exe|Microsoft|w|Windows Subsystem for Linux command.|دستور زیرسیستم لینوکس ویندوز.
wslservice.exe|Microsoft|c|WSL service.|سرویس WSL.
";

pub(crate) fn lookup(name: &str) -> Option<(String, Verdict, String, String)> {
    let n = name.to_lowercase();
    for line in KB.lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() == 5 && f[0] == n {
            return Some((f[1].to_string(), Verdict::parse(f[2]), f[3].to_string(), f[4].to_string()));
        }
    }
    None
}

/// Names that malware likes to copy; real ones live only in System32 / SysWOW64.
const SYSTEM_NAMES: &[&str] = &["svchost.exe", "csrss.exe", "lsass.exe", "smss.exe", "wininit.exe", "winlogon.exe", "services.exe", "dwm.exe", "explorer.exe", "conhost.exe", "taskhostw.exe", "rundll32.exe", "dllhost.exe", "spoolsv.exe"];

/// Built-in answer and checks (no network). `signed`: result of a signature check if the caller made one.
pub fn explain(p: &Proc, signed: Option<bool>) -> Answer {
    let path_l = p.meta.path.to_lowercase();
    let mut notes = vec![];
    let mut suspicious = false;
    let name_l = p.name.to_lowercase();
    if SYSTEM_NAMES.contains(&name_l.as_str()) && !path_l.is_empty() && !(path_l.starts_with(r"c:\windows\system32\") || path_l.starts_with(r"c:\windows\syswow64\") || (name_l == "explorer.exe" && path_l == r"c:\windows\explorer.exe")) {
        suspicious = true;
        notes.push(format!("{} normally runs only from C:\\Windows\\System32, but this one is in {}", p.name, p.meta.path));
    }
    let in_temp = path_l.contains(r"\appdata\local\temp\") || path_l.contains(r"\windows\temp\") || path_l.contains(r"\downloads\");
    if in_temp {
        notes.push("runs from a temporary or downloads folder".into());
    }
    if signed == Some(false) {
        notes.push("the file has no valid digital signature".into());
        if in_temp {
            suspicious = true;
        }
    }
    // a service host: say which services it runs, in Windows' own words (no AI needed)
    if name_l == "svchost.exe" {
        let svcs: Vec<crate::services::Service> = crate::services::list().into_iter().filter(|s| s.pid == p.pid && s.pid != 0).collect();
        if !svcs.is_empty() {
            let first = &svcs[0];
            let desc = crate::services::details(&first.name).map(|d| d.2).unwrap_or_default();
            let desc = desc.split(". ").next().unwrap_or("").trim_end_matches('.').to_string();
            let more = if svcs.len() > 1 { format!(" (+{} more: {})", svcs.len() - 1, svcs[1..].iter().take(3).map(|s| s.display.as_str()).collect::<Vec<_>>().join(", ")) } else { String::new() };
            let what_en = if desc.is_empty() { format!("Runs the Windows service \"{}\"{more}.", first.display) } else { format!("Runs the Windows service \"{}\": {desc}{more}.", first.display) };
            return Answer {
                verdict: if suspicious { Verdict::Suspicious } else { Verdict::Critical },
                what_en,
                what_fa: format!("سرویس ویندوز «{}» را اجرا می‌کند.", first.display),
                maker: "Microsoft".into(),
                notes,
                source: "builtin",
                ends_en: String::new(),
            };
        }
    }
    if let Some((maker, v, en, fa)) = lookup(&p.name) {
        return Answer { verdict: if suspicious { Verdict::Suspicious } else { v }, what_en: en, what_fa: fa, maker, notes, source: "builtin", ends_en: crate::insight::stop_effect(&p.name).unwrap_or_default() };
    }
    // not in the list: describe from the file's own version info
    let desc = &p.meta.description;
    let company = &p.meta.company;
    let verdict = if suspicious {
        Verdict::Suspicious
    } else if p.kind == crate::procs::Kind::App {
        Verdict::Work
    } else if crate::procs::is_critical_name(&p.name) {
        Verdict::Critical
    } else if path_l.starts_with(r"c:\windows\") && signed == Some(true) && company.to_lowercase().contains("microsoft") {
        Verdict::Safe
    } else {
        Verdict::Unknown
    };
    let what_en = if desc.is_empty() { format!("{} (no description in the file)", p.name) } else { desc.clone() };
    let ends_en = match verdict {
        Verdict::Safe if crate::insight::stop_effect(&p.name).is_none() => "A Windows tool. Ending it stops only this tool; Windows keeps running.".to_string(),
        _ => crate::insight::stop_effect(&p.name).unwrap_or_default(),
    };
    Answer { verdict, what_fa: String::new(), what_en, maker: company.clone(), notes, source: "file-info", ends_en }
}

/// Authenticode check of a file (WinVerifyTrust, no network revocation check): its own embedded signature, or else
/// a Windows catalog that lists its hash (most Windows files are catalog-signed). Takes ~5-30 ms.
pub fn is_signed(path: &str) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Security::WinTrust::*;
    let wp = crate::sys::wide(path);
    unsafe {
        let mut file = WINTRUST_FILE_INFO { cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32, pcwszFilePath: windows::core::PCWSTR(wp.as_ptr()), ..Default::default() };
        let mut data = WINTRUST_DATA {
            cbStruct: size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
            dwStateAction: WTD_STATEACTION_VERIFY,
            dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
            ..Default::default()
        };
        let mut guid = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let r = WinVerifyTrust(HWND(-1isize as *mut _), &mut guid, &mut data as *mut _ as *mut _);
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(HWND(-1isize as *mut _), &mut guid, &mut data as *mut _ as *mut _);
        r == 0 || catalog_signed(path)
    }
}

/// Is the file's hash in a signed Windows catalog (and does that catalog verify)?
fn catalog_signed(path: &str) -> bool {
    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::Security::Cryptography::Catalog::*;
    use windows::Win32::Security::WinTrust::*;
    use windows::Win32::Storage::FileSystem::*;
    use windows::core::{PCWSTR, w};
    let wp = crate::sys::wide(path);
    unsafe {
        let Ok(h) = CreateFileW(PCWSTR(wp.as_ptr()), FILE_GENERIC_READ.0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, None) else { return false };
        let mut ok = false;
        // SHA-256 catalogs first (Windows 10+), then SHA-1 ones
        for alg in [w!("SHA256"), PCWSTR::null()] {
            let mut admin = 0isize;
            if CryptCATAdminAcquireContext2(&mut admin, None, alg, None, None).is_err() {
                continue;
            }
            let mut n = 0u32;
            let _ = CryptCATAdminCalcHashFromFileHandle2(admin, h, &mut n, None, None);
            let mut hash = vec![0u8; n.max(1) as usize];
            if n > 0 && CryptCATAdminCalcHashFromFileHandle2(admin, h, &mut n, Some(hash.as_mut_ptr()), None).is_ok() {
                hash.truncate(n as usize);
                let cat = CryptCATAdminEnumCatalogFromHash(admin, &hash, None, None);
                if cat != 0 {
                    let mut info = CATALOG_INFO { cbStruct: size_of::<CATALOG_INFO>() as u32, ..Default::default() };
                    if CryptCATCatalogInfoFromContext(cat, &mut info, 0).is_ok() {
                        let tag: String = hash.iter().map(|b| format!("{b:02X}")).collect();
                        let wtag = crate::sys::wide(&tag);
                        let mut ci = WINTRUST_CATALOG_INFO {
                            cbStruct: size_of::<WINTRUST_CATALOG_INFO>() as u32,
                            pcwszCatalogFilePath: PCWSTR(info.wszCatalogFile.as_ptr()),
                            pcwszMemberTag: PCWSTR(wtag.as_ptr()),
                            pcwszMemberFilePath: PCWSTR(wp.as_ptr()),
                            hMemberFile: h,
                            pbCalculatedFileHash: hash.as_mut_ptr(),
                            cbCalculatedFileHash: hash.len() as u32,
                            hCatAdmin: admin,
                            ..Default::default()
                        };
                        let mut data = WINTRUST_DATA {
                            cbStruct: size_of::<WINTRUST_DATA>() as u32,
                            dwUIChoice: WTD_UI_NONE,
                            fdwRevocationChecks: WTD_REVOKE_NONE,
                            dwUnionChoice: WTD_CHOICE_CATALOG,
                            Anonymous: WINTRUST_DATA_0 { pCatalog: &mut ci },
                            dwStateAction: WTD_STATEACTION_VERIFY,
                            dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
                            ..Default::default()
                        };
                        let mut guid = WINTRUST_ACTION_GENERIC_VERIFY_V2;
                        ok = WinVerifyTrust(HWND(-1isize as *mut _), &mut guid, &mut data as *mut _ as *mut _) == 0;
                        data.dwStateAction = WTD_STATEACTION_CLOSE;
                        WinVerifyTrust(HWND(-1isize as *mut _), &mut guid, &mut data as *mut _ as *mut _);
                    }
                    let _ = CryptCATAdminReleaseCatalogContext(admin, cat, 0);
                }
            }
            let _ = CryptCATAdminReleaseContext(admin, 0);
            if ok {
                break;
            }
        }
        let _ = CloseHandle(h);
        ok
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn signature_is_real() {
        // catalog-signed Windows tools verify; an unsigned exe (our own test binary) does not
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:/Windows".into());
        for f in ["System32/PING.EXE", "System32/notepad.exe"] {
            let p = format!("{root}/{f}");
            if std::path::Path::new(&p).exists() {
                assert!(super::is_signed(&p), "{p}");
            }
        }
        let me = std::env::current_exe().unwrap();
        assert!(!super::is_signed(&me.display().to_string()));
    }
}
