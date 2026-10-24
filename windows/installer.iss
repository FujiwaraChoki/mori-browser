#ifndef AppVersion
  #error AppVersion must be supplied by build-wizard.py
#endif
#ifndef NativeInstaller
  #define NativeInstaller "..\dist\Mori-Setup-" + AppVersion + "-x64.exe"
#endif

[Setup]
AppId=Mori.SetupWizard
AppName=Mori
AppVersion={#AppVersion}
AppPublisher=Mori Browser
AppPublisherURL=https://github.com/FujiwaraChoki/mori-browser
VersionInfoProductName=Mori Installer
VersionInfoDescription=Mori Setup
VersionInfoVersion={#AppVersion}
CreateAppDir=no
Uninstallable=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
WizardStyle=modern
WizardSizePercent=110
SetupIconFile=assets\mori.ico
WizardImageFile=assets\wizard-large.bmp
WizardSmallImageFile=assets\wizard-small.bmp
DisableWelcomePage=no
DisableReadyPage=no
DisableFinishedPage=no
DisableProgramGroupPage=yes
CloseApplications=no
RestartApplications=no
OutputDir=..\dist
OutputBaseFilename=Mori-Setup-{#AppVersion}-x64-Wizard
Compression=none
DiskSpanning=no
SetupLogging=yes

[Messages]
WelcomeLabel1=Welcome to Mori Setup
WelcomeLabel2=This wizard will install Mori on your computer.%n%nClick Next to continue.
FinishedHeadingLabel=Mori Setup Complete
FinishedLabel=Mori has been installed. You can open it from the Start menu.

[Files]
Source: "{#NativeInstaller}"; DestDir: "{tmp}"; DestName: "MoriNativeSetup.exe"; Flags: deleteafterinstall

[Code]
procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if CurStep = ssPostInstall then
  begin
    WizardForm.StatusLabel.Caption := 'Installing Mori...';
    if not Exec(ExpandConstant('{tmp}\MoriNativeSetup.exe'),
      '--silent --do-not-launch-chrome', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
      RaiseException('Mori installation could not start: ' + SysErrorMessage(ResultCode));
    if ResultCode <> 0 then
      RaiseException('Mori installation failed (exit code ' + IntToStr(ResultCode) + ').');
  end;
end;
