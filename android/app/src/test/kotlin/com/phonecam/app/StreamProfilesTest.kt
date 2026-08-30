package com.phonecam.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class StreamProfilesTest {
    @Test
    fun bitrateTableCoversEveryCodecResolutionAndRate() {
        assertEquals(
            4_000_000,
            StreamManager.defaultBitrate(VideoCodec.H264, StreamResolution.HD_720P, 30),
        )
        assertEquals(
            50_000_000,
            StreamManager.defaultBitrate(VideoCodec.H264, StreamResolution.UHD_2160P, 60),
        )
        assertEquals(
            750_000,
            StreamManager.defaultBitrate(VideoCodec.HEVC, StreamResolution.SD_480P, 15),
        )
        assertEquals(
            32_000_000,
            StreamManager.defaultBitrate(VideoCodec.HEVC, StreamResolution.UHD_2160P, 60),
        )

        for (codec in VideoCodec.entries) {
            for (resolution in StreamResolution.entries) {
                val rates = listOf(15, 30, 60).map {
                    StreamManager.defaultBitrate(codec, resolution, it)
                }
                assertTrue("bitrate must increase with frame rate", rates.zipWithNext().all { it.first < it.second })
            }
        }
    }

    @Test
    fun capabilityIntersectionKeepsOnlyExactEncoderTuples() {
        val capture =
            setOf(
                StreamResolution.HD_720P to 30,
                StreamResolution.UHD_2160P to 60,
            )
        val profiles =
            StreamManager.intersectProfiles(capture) { profile, _ ->
                profile.codec == VideoCodec.H264 || profile.width == 1280
            }

        assertTrue(StreamProfile(VideoCodec.H264, 1280, 720, 30) in profiles)
        assertTrue(StreamProfile(VideoCodec.HEVC, 1280, 720, 30) in profiles)
        assertTrue(StreamProfile(VideoCodec.H264, 3840, 2160, 60) in profiles)
        assertFalse(StreamProfile(VideoCodec.HEVC, 3840, 2160, 60) in profiles)
        assertEquals(3, profiles.size)
    }

    @Test
    fun transactionCandidatesPreferRequestedThenSameTupleH264Fallback() {
        val requested = StreamProfile(VideoCodec.HEVC, 3840, 2160, 60)
        val fallback = requested.copy(codec = VideoCodec.H264)
        assertEquals(
            listOf(requested, fallback),
            StreamManager.configurationCandidates(requested, listOf(requested, fallback)),
        )
        assertEquals(
            listOf(requested),
            StreamManager.configurationCandidates(requested, listOf(requested)),
        )
        assertEquals(
            listOf(fallback),
            StreamManager.configurationCandidates(fallback, listOf(requested, fallback)),
        )
    }
}
