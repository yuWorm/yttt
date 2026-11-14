package com.yttt.yai

import android.provider.Settings
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.filled.Bolt
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.HourglassTop
import androidx.compose.material.icons.filled.PanTool
import androidx.compose.material.icons.filled.PauseCircle
import androidx.compose.material.icons.outlined.SmartToy
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import uniffi.yttt_mobile_core.AgentState
import uniffi.yttt_mobile_core.AgentSummary
import uniffi.yttt_mobile_core.WaitingKind

/** An Agent provider's logo, from scripts/build-mobile-agent-logos.py, and whether it keeps its colours. */
fun logoOf(provider: String): Pair<Int, Boolean>? = when (provider) {
    "codex" -> R.drawable.agent_codex to true
    "claude" -> R.drawable.agent_claude to true
    "grok" -> R.drawable.agent_grok to false
    "opencode" -> R.drawable.agent_opencode to false
    "pi" -> R.drawable.agent_pi to false
    "omp" -> R.drawable.agent_omp to false
    else -> null
}

/** The name the desktop gives a provider. */
fun providerName(provider: String): String = when (provider) {
    "codex" -> "Codex"
    "claude" -> "Claude Code"
    "grok" -> "Grok"
    "opencode" -> "OpenCode"
    "pi" -> "Pi"
    "omp" -> "Oh My Pi"
    else -> provider
}

/**
 * The provider's logo: Codex and Claude in their colours, the others in the
 * content colour, as the desktop draws them; a robot for any other provider.
 */
@Composable
fun AgentLogo(provider: String?, size: Dp = 20.dp, modifier: Modifier = Modifier) {
    val logo = provider?.let(::logoOf)
    val description = provider?.let(::providerName)
    when {
        logo == null -> Icon(Icons.Outlined.SmartToy, description, modifier.size(size))
        logo.second -> Image(painterResource(logo.first), description, modifier.size(size))
        else -> Icon(painterResource(logo.first), description, modifier.size(size))
    }
}

/** The logo on a rounded tile, as a list item's leading icon. */
@Composable
fun AgentTile(provider: String?, size: Dp = 22.dp) {
    Surface(shape = RoundedCornerShape(10.dp), color = MaterialTheme.colorScheme.surfaceContainerHighest) {
        Box(Modifier.padding(7.dp), contentAlignment = Alignment.Center) { AgentLogo(provider, size) }
    }
}

/** Whether an Agent needs the user: waiting, or failed. */
val AgentSummary.urgent: Boolean get() = state == AgentState.WAITING || state == AgentState.FAILED

/** The icon beside an Agent's state: a raised hand for approval, a bubble for a question, a bolt while working. */
val AgentSummary.stateIcon: ImageVector
    get() = when (state) {
        AgentState.WAITING -> if (waiting == WaitingKind.USER_INPUT) Icons.AutoMirrored.Filled.Chat else Icons.Filled.PanTool
        AgentState.FAILED -> Icons.Filled.Error
        AgentState.WORKING -> Icons.Filled.Bolt
        AgentState.STARTING -> Icons.Filled.HourglassTop
        AgentState.IDLE -> Icons.Filled.PauseCircle
        AgentState.COMPLETED, AgentState.INTERRUPTED, AgentState.STALE -> Icons.Filled.CheckCircle
    }

/** Working or starting: shown in blue, with a light sweeping across. */
val AgentSummary.running: Boolean
    get() = state == AgentState.WORKING || state == AgentState.STARTING

/** The blue of running work, lighter on a dark background. */
val runningBlue: Color
    @Composable get() =
        if (MaterialTheme.colorScheme.background.luminance() < 0.5f) Color(0xFF64B5F6) else Color(0xFF1E6FD9)

@Composable
fun AgentSummary.stateColor(): Color = when {
    urgent -> MaterialTheme.colorScheme.error
    running -> runningBlue
    else -> MaterialTheme.colorScheme.onSurfaceVariant
}

/**
 * A light sweeping across what is drawn, over and over, while [active]; still
 * when the system turns animations off.
 */
@Composable
fun Modifier.shimmer(active: Boolean = true): Modifier {
    if (!active) return this
    val context = LocalContext.current
    val animationsOn = remember {
        Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) > 0f
    }
    if (!animationsOn) return this
    val phase by rememberInfiniteTransition(label = "shimmer").animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(1400, easing = LinearEasing), RepeatMode.Restart),
        label = "shimmer-phase",
    )
    return graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
        .drawWithContent {
            drawContent()
            val band = size.width * 0.6f
            val start = -band + phase * (size.width + band)
            drawRect(
                brush = Brush.horizontalGradient(
                    listOf(Color.Transparent, Color.White.copy(alpha = 0.9f), Color.Transparent),
                    startX = start,
                    endX = start + band,
                ),
                blendMode = BlendMode.SrcAtop,
            )
        }
}

/** An Agent's state as a coloured icon and word; a running one shimmers. */
@Composable
fun AgentStateLabel(agent: AgentSummary) {
    Row(
        Modifier.shimmer(agent.running),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(3.dp),
    ) {
        Icon(agent.stateIcon, null, Modifier.size(16.dp), tint = agent.stateColor())
        Text(agent.stateLabel, color = agent.stateColor(), fontWeight = FontWeight.Medium,
            style = MaterialTheme.typography.labelMedium)
    }
}
