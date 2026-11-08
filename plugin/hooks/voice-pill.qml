// Floating pill for AgentSI voice mode (S025). Run as `qs -p voice-pill.qml` by voice-mode.mjs.
// In, on stdin, one JSON line at a time:
//   {"state":"preparing"}                                  a reply is being prepared
//   {"state":"playing","start":<epoch ms>,"envelope":[…]}  level per 1/30 s of audio, from the first word
//   {"state":"closed"}                                     close the pill
// Out, on stdout: {"event":"stop"} when the pill is clicked.
// qs starts the Process children below with this qs as their parent, so $PPID is qs and its fd 0 and 1 are the pipes.
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland

ShellRoot {
    id: root
    property string mode: "preparing"
    property var envelope: []
    property real startMs: 0
    property real nowMs: Date.now()
    property real level: {
        if (mode !== "playing" || envelope.length === 0) return 0;
        var i = Math.floor((nowMs - startMs) / 1000 * 30);
        return i >= 0 && i < envelope.length ? envelope[i] : 0;
    }

    Process {
        running: true
        command: ["sh", "-c", "exec cat /proc/$PPID/fd/0"]
        stdout: SplitParser {
            onRead: function (line) {
                var msg;
                try { msg = JSON.parse(line); } catch (e) { return; }
                if (msg.state === "closed") Qt.quit();
                else if (msg.state === "preparing") root.mode = "preparing";
                else if (msg.state === "playing") {
                    root.envelope = msg.envelope || [];
                    root.startMs = msg.start || Date.now();
                    root.mode = "playing";
                }
            }
        }
        onExited: Qt.quit()
    }

    Process {
        id: stopSignal
        command: ["sh", "-c", "printf '{\"event\":\"stop\"}\\n' > /proc/$PPID/fd/1"]
    }

    Timer {
        interval: 33
        running: true
        repeat: true
        onTriggered: root.nowMs = Date.now()
    }

    PanelWindow {
        anchors.top: true
        margins.top: 12
        implicitWidth: 168
        implicitHeight: 40
        color: "transparent"
        exclusiveZone: 0
        WlrLayershell.layer: WlrLayer.Overlay
        WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

        Rectangle {
            anchors.fill: parent
            radius: height / 2
            color: "#e61e1e2e"
            border.color: "#45475a"

            Row {
                anchors.centerIn: parent
                spacing: 4
                Repeater {
                    model: 9
                    Rectangle {
                        width: 6
                        radius: 3
                        anchors.verticalCenter: parent.verticalCenter
                        color: "#cba6f7"
                        property real wave: 0.5 + 0.5 * Math.sin(root.nowMs / 180 + index * 0.8)
                        height: root.mode === "playing" ? 6 + 24 * Math.min(1, root.level * (0.6 + 0.4 * Math.abs(Math.sin(index * 1.7)))) : 6 + 10 * wave
                        opacity: root.mode === "playing" ? 1 : 0.6
                    }
                }
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: stopSignal.running = true
            }
        }
    }
}
