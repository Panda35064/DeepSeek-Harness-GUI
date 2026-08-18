!macro NSIS_HOOK_POSTUNINSTALL
  ; 勾选“删除应用程序数据”时，额外删除客户端实际使用的数据目录
  ; （%APPDATA%\DeepSeek Harness，包含便携 Node.js runtime、npm 全局包、
  ;  config.json 和日志），避免卸载后残留。
  ; 为加快删除速度：先把目录改名（原路径立即消失），再调用系统 rmdir
  ; 在后台快速递归删除，避免 NSIS 逐文件删除导致卸载长时间卡住。
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    ${If} ${FileExists} "$APPDATA\DeepSeek Harness"
      ExecWait 'cmd.exe /c rmdir /s /q "$APPDATA\DeepSeek Harness.old"'
      Rename "$APPDATA\DeepSeek Harness" "$APPDATA\DeepSeek Harness.old"
      Exec 'cmd.exe /c rmdir /s /q "$APPDATA\DeepSeek Harness.old"'
    ${EndIf}
    ${If} ${FileExists} "$LOCALAPPDATA\DeepSeek Harness"
      ExecWait 'cmd.exe /c rmdir /s /q "$LOCALAPPDATA\DeepSeek Harness.old"'
      Rename "$LOCALAPPDATA\DeepSeek Harness" "$LOCALAPPDATA\DeepSeek Harness.old"
      Exec 'cmd.exe /c rmdir /s /q "$LOCALAPPDATA\DeepSeek Harness.old"'
    ${EndIf}
  ${EndIf}
!macroend
