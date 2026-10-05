; Inno Setup script for BergPDF (https://jrsoftware.org/isinfo.php).
; Build the folder first:   cargo xtask dist            (add --with-ocr-models to bundle OCR, see README)
; Then:   ISCC.exe packaging\windows\bergpdf.iss /DAppVersion=0.1.0
; Result: dist\BergPDF-Setup-<version>.exe (unsigned; not yet tried on Windows, see docs/PLATFORM_CHECKLIST.md).

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\dist\bergpdf-" + AppVersion + "-windows-x86_64"
#endif

[Setup]
AppId={{A3142D40-8DD1-4B29-9C55-55BEF3F3C345}
AppName=BergPDF
AppVersion={#AppVersion}
AppPublisher=BergPDF
DefaultDirName={autopf}\BergPDF
DefaultGroupName=BergPDF
DisableProgramGroupPage=yes
OutputDir=..\..\dist
OutputBaseFilename=BergPDF-Setup-{#AppVersion}
LicenseFile={#SourceDir}\LICENSE
SetupIconFile={#SourceDir}\bergpdf.ico
UninstallDisplayIcon={app}\bergpdf.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Per-user install without administrator rights; the wizard offers "for all users" as a choice.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "openwith"; Description: "Show BergPDF in the ""Open with"" list for PDF files"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{autoprograms}\BergPDF"; Filename: "{app}\bergpdf.exe"
Name: "{autodesktop}\BergPDF"; Filename: "{app}\bergpdf.exe"; Tasks: desktopicon

[Registry]
; Does not make BergPDF the default PDF app (Windows asks the user for that); it only lists it.
Root: HKA; Subkey: "Software\Classes\Applications\bergpdf.exe\shell\open\command"; ValueType: string; ValueData: """{app}\bergpdf.exe"" ""%1"""; Tasks: openwith; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\.pdf\OpenWithList\bergpdf.exe"; ValueType: none; Tasks: openwith; Flags: uninsdeletekey

[Run]
Filename: "{app}\bergpdf.exe"; Description: "Start BergPDF"; Flags: nowait postinstall skipifsilent
