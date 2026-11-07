// The primary action of a card: a button in the accent color, as in the
// mockups. The Plasma button style has no highlighted state.
import QtQuick
import org.kde.plasma.components as PlasmaComponents3
import org.kde.kirigami as Kirigami

PlasmaComponents3.Button {
    id: button

    property color accent: Kirigami.Theme.highlightColor

    // The accent background has no size of its own: take the height of a
    // plain button, so that the primary action lines up with the others.
    implicitHeight: ruler.implicitHeight
    font.weight: Font.DemiBold
    palette.buttonText: Kirigami.Theme.highlightedTextColor
    background: G2Rect {
        radius: Kirigami.Units.cornerRadius
        opacity: button.enabled ? 1 : 0.5
        color: button.down ? Qt.darker(button.accent, 1.15)
             : button.hovered ? Qt.lighter(button.accent, 1.2)
             : button.accent
        borderColor: Kirigami.Theme.focusColor
        borderWidth: button.visualFocus ? 2 : 0
    }

    PlasmaComponents3.Button {
        id: ruler
        visible: false
        text: button.text
    }
}
