package dev.yttt.spike

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uniffi.yttt_mobile_core.TerminalKey

/**
 * The keys a phone keyboard cannot type, above it while the user controls
 * the terminal: Esc, Tab, a one-shot Ctrl, the arrows, Enter, and the
 * keyboard itself; Shift-Tab and Backspace after a scroll.
 */
@Composable
fun KeyBar(
    ctrlArmed: Boolean,
    keyboardShown: Boolean,
    onKey: (TerminalKey) -> Unit,
    onCtrl: () -> Unit,
    onKeyboard: () -> Unit,
) {
    Surface(color = MaterialTheme.colorScheme.surfaceContainer, modifier = Modifier.fillMaxWidth()) {
        Row(
            Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 4.dp, vertical = 2.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            KeyButton("Esc") { onKey(TerminalKey.ESCAPE) }
            KeyButton("Tab") { onKey(TerminalKey.TAB) }
            FilterChip(selected = ctrlArmed, onClick = onCtrl, label = { Text("Ctrl") })
            KeyButton("←") { onKey(TerminalKey.LEFT) }
            KeyButton("↓") { onKey(TerminalKey.DOWN) }
            KeyButton("↑") { onKey(TerminalKey.UP) }
            KeyButton("→") { onKey(TerminalKey.RIGHT) }
            KeyButton("⏎") { onKey(TerminalKey.ENTER) }
            FilterChip(selected = keyboardShown, onClick = onKeyboard, label = { Text("⌨") })
            KeyButton("⇧Tab") { onKey(TerminalKey.SHIFT_TAB) }
            KeyButton("⌫") { onKey(TerminalKey.BACKSPACE) }
        }
    }
}

@Composable
private fun KeyButton(label: String, onClick: () -> Unit) {
    FilledTonalButton(onClick = onClick, contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 12.dp)) {
        Text(label)
    }
}
