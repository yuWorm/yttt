package com.yttt.yai

import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.yttt_mobile_core.LinkState

class LinkBannerTest {
    @Test
    fun theBannerSaysWhatTheLinkDoes() {
        assertEquals("", LinkBannerText.title(LinkState.Ready))
        assertEquals("正在连接…", LinkBannerText.title(LinkState.Connecting))
        assertEquals("连接断开，正在重连…", LinkBannerText.title(LinkState.Reconnecting(1u, "reset")))
        assertEquals("连接断开，正在重连（第 3 次）…", LinkBannerText.title(LinkState.Reconnecting(3u, "reset")))
        assertEquals("连接已断开：the Host refused", LinkBannerText.title(LinkState.Lost("the Host refused")))
        assertEquals(15_000L, LinkBannerText.FRESH_AFTER_MILLIS)
    }
}
