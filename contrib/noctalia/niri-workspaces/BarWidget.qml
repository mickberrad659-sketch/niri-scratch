import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Services.Compositor
import qs.Widgets

Item {
  id: root
  property var pluginApi: null
  property ShellScreen screen
  property string widgetId: ""
  property string section: ""
  property int sectionWidgetIndex: -1
  property int sectionWidgetsCount: 0

  readonly property string screenName: screen ? screen.name : ""
  readonly property real barHeight: Style.getBarHeightForScreen(screenName)
  readonly property real capsuleHeight: Style.getCapsuleHeightForScreen(screenName)
  property ListModel normalWorkspaces: ListModel {}
  property string activeScratch: ""
  property int returnWorkspaceId: -1
  property int lastNormalWorkspaceId: -1
  property bool statusRefreshPending: false

  readonly property var scratchIcons: ({
    "web": "brand-firefox",
    "d": "letter-d",
    "f": "letter-f",
    "chat": "brand-telegram",
    "u": "letter-u"
  })

  implicitWidth: capsule.implicitWidth
  implicitHeight: barHeight

  function focusedWorkspace() {
    for (var i = 0; i < CompositorService.workspaces.count; i++) {
      var ws = CompositorService.workspaces.get(i);
      if (ws.isFocused)
        return ws;
    }
    return null;
  }

  function scheduleRefresh() {
    var focused = focusedWorkspace();
    if (focused && focused.name && focused.name.indexOf("scratch:") === 0) {
      activeScratch = focused.name.substring(8);
      if (!statusProcess.running) {
        statusProcess.command = [Quickshell.env("HOME") + "/.local/bin/niri-scratch", "--json", "status"];
        statusProcess.running = true;
      } else {
        statusRefreshPending = true;
      }
    } else {
      activeScratch = "";
      returnWorkspaceId = -1;
      if (focused && Number(focused.name) >= 1 && Number(focused.name) <= 6)
        lastNormalWorkspaceId = focused.id;
    }
    rebuild();
  }

  function rebuild() {
    var next = [];
    var targetId = returnWorkspaceId > 0 ? returnWorkspaceId : lastNormalWorkspaceId;
    for (var i = 0; i < CompositorService.workspaces.count; i++) {
      var ws = CompositorService.workspaces.get(i);
      var number = Number(ws.name);
      if (Number.isInteger(number) && number >= 1 && number <= 6) {
        next.push({
          "workspaceId": ws.id,
          "workspaceIdx": ws.idx,
          "workspaceName": ws.name,
          "workspaceOutput": ws.output,
          "isFocused": ws.isFocused,
          "scratchName": ""
        });
      }
    }
    next.sort(function(a, b) { return Number(a.workspaceName) - Number(b.workspaceName); });
    if (activeScratch) {
      var targetExists = next.some(function(ws) { return ws.workspaceId === targetId; });
      if (!targetExists && next.length > 0)
        targetId = next[0].workspaceId;
      for (var k = 0; k < next.length; k++) {
        if (next[k].workspaceId === targetId)
          next[k].scratchName = activeScratch;
      }
    }
    normalWorkspaces.clear();
    for (var j = 0; j < next.length; j++)
      normalWorkspaces.append(next[j]);
  }

  function applyStatus(text) {
    var focused = focusedWorkspace();
    if (!focused || !focused.name || focused.name.indexOf("scratch:") !== 0) {
      activeScratch = "";
      returnWorkspaceId = -1;
      rebuild();
      return;
    }
    var focusedScratch = focused.name.substring(8);
    try {
      var response = JSON.parse(text);
      var data = response.data || {};
      for (var name in data) {
        if (data[name].visible && name === focusedScratch) {
          activeScratch = name;
          var origin = data[name].origin;
          if (origin && origin.workspace_id)
            returnWorkspaceId = origin.workspace_id;
          break;
        }
      }
    } catch (error) {
      Logger.w("niri-workspaces", "Could not parse niri-scratch status: " + error);
    }
    rebuild();
  }

  function activate(workspaceId, workspaceIdx, workspaceName, workspaceOutput, scratchName) {
    if (scratchName) {
      toggleProcess.command = [Quickshell.env("HOME") + "/.local/bin/niri-scratch", "toggle", scratchName];
      toggleProcess.running = true;
      return;
    }
    CompositorService.switchToWorkspace({
      "id": workspaceId,
      "idx": workspaceIdx,
      "name": workspaceName,
      "output": workspaceOutput
    });
  }

  Component.onCompleted: Qt.callLater(scheduleRefresh)

  Connections {
    target: CompositorService
    function onWorkspacesChanged() { Qt.callLater(root.scheduleRefresh); }
  }

  Process {
    id: statusProcess
    running: false
    stdout: StdioCollector {
      onStreamFinished: root.applyStatus(text)
    }
    onExited: function() {
      if (root.statusRefreshPending) {
        root.statusRefreshPending = false;
        statusProcess.running = true;
      }
    }
  }

  Process {
    id: toggleProcess
    running: false
  }

  Rectangle {
    id: capsule
    anchors.centerIn: parent
    implicitWidth: row.implicitWidth + Style.marginM * 2
    implicitHeight: root.capsuleHeight
    radius: Style.radiusL
    color: root.activeScratch ? Qt.alpha(Color.mSecondary, 0.22) : Style.capsuleColor
    border.color: root.activeScratch ? Color.mSecondary : Style.capsuleBorderColor
    border.width: Style.capsuleBorderWidth

    RowLayout {
      id: row
      anchors.centerIn: parent
      spacing: Style.marginXS

      Repeater {
        model: normalWorkspaces

        Rectangle {
          required property int workspaceId
          required property int workspaceIdx
          required property string workspaceName
          required property string workspaceOutput
          required property bool isFocused
          required property string scratchName

          implicitWidth: root.capsuleHeight * ((isFocused || scratchName) ? 0.90 : 0.62)
          implicitHeight: root.capsuleHeight * 0.56
          radius: height / 2
          color: scratchName ? Color.mSecondary
                             : (isFocused ? Color.mPrimary : "transparent")

          NIcon {
            anchors.centerIn: parent
            visible: parent.scratchName.length > 0
            icon: root.scratchIcons[parent.scratchName] || "apps"
            color: Color.mOnSecondary
            applyUiScale: false
          }

          NText {
            anchors.centerIn: parent
            visible: parent.scratchName.length === 0
            text: parent.workspaceName
            color: parent.isFocused ? Color.mOnPrimary : Color.mOnSurface
            applyUiScale: false
          }

          MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: root.activate(parent.workspaceId, parent.workspaceIdx,
                                     parent.workspaceName, parent.workspaceOutput,
                                     parent.scratchName)
          }
        }
      }
    }
  }
}
