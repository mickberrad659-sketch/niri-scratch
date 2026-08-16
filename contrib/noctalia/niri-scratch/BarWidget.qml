import QtQuick
import QtQuick.Layouts
import Quickshell
import qs.Commons
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
  readonly property var pads: [
    { "name": "web", "icon": "brand-firefox" },
    { "name": "d", "icon": "letter-d" },
    { "name": "f", "icon": "letter-f" },
    { "name": "chat", "icon": "brand-telegram" },
    { "name": "u", "icon": "letter-u" }
  ]

  implicitWidth: icons.implicitWidth + Style.marginM * 2
  implicitHeight: capsuleHeight

  Rectangle {
    anchors.fill: parent
    radius: Style.radiusL
    color: Style.capsuleColor
    border.color: Style.capsuleBorderColor
    border.width: Style.capsuleBorderWidth

    RowLayout {
      id: icons
      anchors.centerIn: parent
      spacing: Style.marginXS

      Repeater {
        model: root.pads
        Item {
          required property var modelData
          implicitWidth: root.capsuleHeight * 0.72
          implicitHeight: root.capsuleHeight
          NIcon {
            anchors.centerIn: parent
            icon: modelData.icon
            applyUiScale: false
            color: padMouse.containsMouse ? Color.mPrimary : Color.mOnSurface
          }
          MouseArea {
            id: padMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: if (root.pluginApi?.mainInstance) root.pluginApi.mainInstance.toggle(parent.modelData.name)
          }
        }
      }
    }
  }
}
