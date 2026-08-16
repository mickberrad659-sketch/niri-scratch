import QtQuick
import QtQuick.Layouts
import Quickshell
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
  readonly property real capsuleHeight: Style.getCapsuleHeightForScreen(screenName)
  property ListModel normalWorkspaces: ListModel {}
  implicitWidth: row.implicitWidth + Style.marginM * 2
  implicitHeight: capsuleHeight

  function refresh() {
    var next = [];
    for (var i = 0; i < CompositorService.workspaces.count; i++) {
      var ws = CompositorService.workspaces.get(i);
      var number = Number(ws.name);
      if (Number.isInteger(number) && number >= 1 && number <= 6)
        next.push({ "id": ws.id, "idx": ws.idx, "name": ws.name,
                    "isFocused": ws.isFocused, "isOccupied": ws.isOccupied,
                    "output": ws.output });
    }
    next.sort(function(a, b) { return Number(a.name) - Number(b.name); });
    normalWorkspaces.clear();
    for (var j = 0; j < next.length; j++) normalWorkspaces.append(next[j]);
  }

  Component.onCompleted: Qt.callLater(refresh)
  Connections {
    target: CompositorService
    function onWorkspacesChanged() { Qt.callLater(root.refresh); }
  }
  Rectangle {
    anchors.fill: parent
    radius: Style.radiusL
    color: Style.capsuleColor
    border.color: Style.capsuleBorderColor
    border.width: Style.capsuleBorderWidth
    RowLayout {
      id: row
      anchors.centerIn: parent
      spacing: Style.marginXS
      Repeater {
        model: normalWorkspaces
        Rectangle {
          required property int id
          required property int idx
          required property string name
          required property bool isFocused
          required property bool isOccupied
          required property string output
          implicitWidth: root.capsuleHeight * (isFocused ? 1.15 : 0.72)
          implicitHeight: root.capsuleHeight * 0.72
          radius: height / 2
          color: isFocused ? Color.mPrimary : Color.mSurfaceVariant
          NText {
            anchors.centerIn: parent
            text: name
            color: isFocused ? Color.mOnPrimary : Color.mOnSurface
            applyUiScale: false
          }
          MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: CompositorService.switchToWorkspace({ "id": parent.id, "idx": parent.idx,
                                                               "name": parent.name, "output": parent.output })
          }
        }
      }
    }
  }
}
