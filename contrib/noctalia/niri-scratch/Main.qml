import QtQuick
import Quickshell
import Quickshell.Io
import qs.Services.UI

Item {
  id: root
  property var pluginApi: null
  property string pendingScratchpad: ""

  function toggle(name) {
    if (!name || toggleProcess.running)
      return;
    pendingScratchpad = name;
    toggleProcess.command = [Quickshell.env("HOME") + "/.local/bin/niri-scratch", "toggle", name];
    toggleProcess.running = true;
  }

  Process {
    id: toggleProcess
    running: false
    onExited: function(code) {
      if (code !== 0)
        ToastService.showError("Scratchpad '" + root.pendingScratchpad + "' failed");
      root.pendingScratchpad = "";
    }
  }

  IpcHandler {
    target: "plugin:niri-scratch"
    function toggle(name: string) { root.toggle(name); }
  }
}
