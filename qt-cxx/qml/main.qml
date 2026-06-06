import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.example.gitreview 1.0

ApplicationWindow {
    id: win
    visible: true
    width: 1200
    height: 820
    title: "git-review · cxx-qt"
    color: "#ffffff"

    // The CXX-Qt QObject, registered as a QML type via #[qml_element]. It opens the repo and
    // builds the model in its Rust initialize() hook, then exposes everything as properties.
    Backend { id: backend }

    // ---- palette ----
    readonly property color cPanel: "#f6f8fa"
    readonly property color cBorder: "#d0d7de"
    readonly property color cText: "#1f2328"
    readonly property color cMuted: "#656d76"
    readonly property color cAccent: "#0969da"
    readonly property color cSel: "#ddf4ff"
    readonly property color cAddBg: "#e6ffec"
    readonly property color cAddMark: "#abf2bc"
    readonly property color cDelBg: "#ffebe9"
    readonly property color cDelMark: "#ff8182"
    readonly property color cAddFg: "#1a7f37"
    readonly property color cDelFg: "#cf222e"
    readonly property color cHunk: "#ddf4ff"

    // ---- models parsed from the Rust backend's JSON ----
    property var commits: JSON.parse(backend.commitsJson)
    property var files: JSON.parse(backend.filesJson)
    property var diffItems: JSON.parse(backend.diffJson)
    property int lineH: Math.round(backend.font * 1.6)

    // counts for a file path, used by the sticky section header
    function fileCounts(p) {
        for (var i = 0; i < files.length; i++)
            if (files[i].path === p) return files[i];
        return null;
    }

    SplitView {
        anchors.fill: parent
        orientation: Qt.Horizontal

        // ===== LEFT SIDE PANEL =====
        SplitView {
            orientation: Qt.Vertical
            SplitView.preferredWidth: 320
            SplitView.minimumWidth: 220
            SplitView.maximumWidth: 600

            // ---- commit list ----
            Rectangle {
                SplitView.preferredHeight: 360
                SplitView.minimumHeight: 120
                color: cPanel
                ColumnLayout {
                    anchors.fill: parent
                    spacing: 0
                    Label {
                        text: "COMMITS · " + backend.repoName
                        color: cMuted
                        font.pixelSize: 10
                        font.bold: true
                        Layout.margins: 6
                    }
                    ListView {
                        id: commitList
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: win.commits
                        ScrollBar.vertical: ScrollBar {}
                        delegate: Rectangle {
                            width: ListView.view.width
                            height: 52
                            color: modelData.current ? cSel : "transparent"
                            Rectangle { anchors.bottom: parent.bottom; width: parent.width
                                height: 1; color: cBorder }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: backend.select_commit(index)
                            }
                            ColumnLayout {
                                anchors.fill: parent
                                anchors.leftMargin: 8
                                anchors.rightMargin: 8
                                anchors.topMargin: 5
                                spacing: 3
                                RowLayout {
                                    spacing: 4
                                    Layout.fillWidth: true
                                    Rectangle {
                                        visible: !modelData.working
                                        width: 18; height: 18; radius: 4
                                        color: modelData.isFrom ? cAccent : "#eaeef2"
                                        border.width: 1; border.color: cBorder
                                        Text { anchors.centerIn: parent; text: "◀"; font.pixelSize: 9
                                            color: modelData.isFrom ? "white" : cMuted }
                                        MouseArea { anchors.fill: parent; onClicked: backend.set_from(index) }
                                    }
                                    Rectangle {
                                        visible: !modelData.working
                                        width: 18; height: 18; radius: 4
                                        color: modelData.isTo ? cAccent : "#eaeef2"
                                        border.width: 1; border.color: cBorder
                                        Text { anchors.centerIn: parent; text: "▶"; font.pixelSize: 9
                                            color: modelData.isTo ? "white" : cMuted }
                                        MouseArea { anchors.fill: parent; onClicked: backend.set_to(index) }
                                    }
                                    Text { text: modelData.short; color: cAccent
                                        font.family: "monospace"; font.pixelSize: 11 }
                                    Text { text: modelData.date; color: cMuted; font.pixelSize: 10 }
                                    Item { Layout.fillWidth: true }
                                    Text { text: modelData.author; color: cMuted; font.pixelSize: 10
                                        elide: Text.ElideRight; Layout.maximumWidth: 110 }
                                }
                                Text { text: modelData.title; color: cText; font.pixelSize: 12
                                    elide: Text.ElideRight; Layout.fillWidth: true }
                            }
                        }
                    }
                }
            }

            // ---- file tree ----
            Rectangle {
                color: cPanel
                ColumnLayout {
                    anchors.fill: parent
                    spacing: 0
                    Label {
                        text: "FILES (" + win.files.length + ")"
                        color: cMuted; font.pixelSize: 10; font.bold: true
                        Layout.margins: 6
                    }
                    ListView {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: win.files
                        ScrollBar.vertical: ScrollBar {}
                        delegate: Rectangle {
                            width: ListView.view.width
                            height: 26
                            color: fileHover.hovered ? "#eaeef2" : "transparent"
                            HoverHandler { id: fileHover }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: diffList.positionViewAtIndex(modelData.itemIndex, ListView.Beginning)
                            }
                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: 8
                                anchors.rightMargin: 8
                                spacing: 6
                                Text { text: modelData.icon; font.pixelSize: 13 }
                                Text { text: modelData.path; color: cText; font.pixelSize: 12
                                    elide: Text.ElideRight; Layout.fillWidth: true }
                                Text { text: "+" + modelData.added; color: cAddFg; font.pixelSize: 11 }
                                Text { text: "−" + modelData.removed; color: cDelFg; font.pixelSize: 11 }
                            }
                        }
                    }
                }
            }
        }

        // ===== MAIN VIEW =====
        ColumnLayout {
            SplitView.fillWidth: true
            spacing: 0

            // ---- toolbar ----
            Rectangle {
                Layout.fillWidth: true
                height: 40
                color: "#ffffff"
                Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: cBorder }
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 10
                    anchors.rightMargin: 8
                    spacing: 8
                    Text { text: backend.summary; color: cText; font.family: "monospace"
                        font.bold: true; font.pixelSize: 13 }
                    Text { text: "+" + backend.totalAdded; color: cAddFg; font.pixelSize: 13 }
                    Text { text: "−" + backend.totalRemoved; color: cDelFg; font.pixelSize: 13 }
                    Item { Layout.fillWidth: true }
                    ToolButton {
                        text: "⤶"; checkable: true; checked: backend.wrap
                        ToolTip.text: "Word wrap"; ToolTip.visible: hovered; ToolTip.delay: 400
                        onClicked: backend.toggle_wrap()
                    }
                    ToolButton {
                        text: "␣"; checkable: true; checked: backend.showSpace
                        ToolTip.text: "Show space changes"; ToolTip.visible: hovered; ToolTip.delay: 400
                        onClicked: backend.toggle_space()
                    }
                    ToolButton {
                        text: "A-"; ToolTip.text: "Decrease font size"; ToolTip.visible: hovered
                        ToolTip.delay: 400; onClicked: backend.font_dec()
                    }
                    ToolButton {
                        text: "A+"; ToolTip.text: "Increase font size"; ToolTip.visible: hovered
                        ToolTip.delay: 400; onClicked: backend.font_inc()
                    }
                    ToolButton {
                        text: "#"; checkable: true; checked: backend.lineNumbers
                        ToolTip.text: "Show line numbers"; ToolTip.visible: hovered; ToolTip.delay: 400
                        onClicked: backend.toggle_line_numbers()
                    }
                }
            }

            // ---- diff body ----
            ListView {
                id: diffList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: win.diffItems
                ScrollBar.vertical: ScrollBar {}
                ScrollBar.horizontal: ScrollBar { policy: backend.wrap ? ScrollBar.AlwaysOff : ScrollBar.AsNeeded }
                contentWidth: backend.wrap ? width : 2400

                // sticky per-file header via list sections
                section.property: "file"
                section.criteria: ViewSection.FullString
                section.delegate: Rectangle {
                    id: sectHdr
                    width: ListView.view.width
                    height: section === "" ? 0 : 30
                    visible: section !== ""
                    color: cPanel
                    property var fc: win.fileCounts(section)
                    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: cBorder }
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 10
                        anchors.rightMargin: 10
                        spacing: 8
                        Text { text: section; color: cText; font.bold: true
                            font.pixelSize: 12; elide: Text.ElideRight; Layout.fillWidth: true }
                        Text { text: sectHdr.fc ? "+" + sectHdr.fc.added : ""
                            color: cAddFg; font.pixelSize: 12 }
                        Text { text: sectHdr.fc ? "−" + sectHdr.fc.removed : ""
                            color: cDelFg; font.pixelSize: 12 }
                    }
                }

                delegate: Item {
                    width: diffList.contentWidth
                    height: modelData.kind === 2 ? msgCol.implicitHeight + 24 : win.lineH

                    // commit-message block
                    ColumnLayout {
                        id: msgCol
                        visible: modelData.kind === 2
                        width: diffList.width - 24
                        x: 12; y: 12
                        spacing: 3
                        Text { text: modelData.title ? modelData.title : ""; color: cText
                            font.pixelSize: 17; font.bold: true; wrapMode: Text.WordWrap
                            Layout.fillWidth: true }
                        Text { text: modelData.sub ? modelData.sub : ""; color: cMuted; font.pixelSize: 11 }
                        Text { text: modelData.body ? modelData.body : ""; color: cText
                            font.family: "monospace"; font.pixelSize: 12; wrapMode: Text.WordWrap
                            Layout.fillWidth: true }
                    }

                    // code line
                    Rectangle {
                        visible: modelData.kind === 0
                        anchors.fill: parent
                        color: modelData.codeKind === 1 ? cAddBg
                            : modelData.codeKind === 2 ? cDelBg
                            : modelData.codeKind === 3 ? cHunk : "transparent"
                        Row {
                            anchors.fill: parent
                            Text {
                                visible: backend.lineNumbers && modelData.codeKind !== 3
                                width: backend.font * 2.6; height: parent.height
                                text: modelData.oldNo ? modelData.oldNo : ""
                                color: cMuted; font.family: "monospace"; font.pixelSize: backend.font
                                horizontalAlignment: Text.AlignRight; verticalAlignment: Text.AlignVCenter
                                rightPadding: 4
                            }
                            Text {
                                visible: backend.lineNumbers && modelData.codeKind !== 3
                                width: backend.font * 2.6; height: parent.height
                                text: modelData.newNo ? modelData.newNo : ""
                                color: cMuted; font.family: "monospace"; font.pixelSize: backend.font
                                horizontalAlignment: Text.AlignRight; verticalAlignment: Text.AlignVCenter
                                rightPadding: 4
                            }
                            Rectangle {
                                width: backend.font * 1.4; height: parent.height
                                color: modelData.codeKind === 1 ? cAddMark
                                    : modelData.codeKind === 2 ? cDelMark : "transparent"
                                Text { anchors.centerIn: parent; text: modelData.sign ? modelData.sign : ""
                                    color: modelData.codeKind === 1 ? cAddFg
                                        : modelData.codeKind === 2 ? cDelFg : cMuted
                                    font.family: "monospace"; font.pixelSize: backend.font }
                            }
                            Text {
                                height: parent.height
                                leftPadding: 6
                                text: modelData.html
                                textFormat: Text.RichText
                                font.family: "monospace"; font.pixelSize: backend.font
                                color: cText
                                verticalAlignment: Text.AlignVCenter
                                wrapMode: backend.wrap ? Text.WrapAnywhere : Text.NoWrap
                            }
                        }
                    }
                }
            }
        }
    }
}
