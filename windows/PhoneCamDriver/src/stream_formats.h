#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <limits>

namespace phonecam {

struct Resolution {
    std::uint32_t width;
    std::uint32_t height;
};

inline constexpr std::array<Resolution, 5> kResolutions = {{
    {640, 480},
    {1280, 720},
    {1920, 1080},
    {2560, 1440},
    {3840, 2160},
}};
inline constexpr std::array<std::uint32_t, 3> kFrameRates = {15, 30, 60};
inline constexpr std::array<std::int64_t, 3> kFrameDurations100ns = {666667, 333333, 166667};
inline constexpr std::size_t kMediaTypeCount = kResolutions.size() * kFrameRates.size();
inline constexpr std::uint64_t kMaximumNv12Bytes = 12'441'600;

constexpr bool IsSupportedDimensions(std::uint32_t width, std::uint32_t height) {
    for (const auto& resolution : kResolutions) {
        if (resolution.width == width && resolution.height == height) return true;
    }
    return false;
}

constexpr bool IsSupportedFrameDuration(std::int64_t duration) {
    for (const auto candidate : kFrameDurations100ns) {
        if (candidate == duration) return true;
    }
    return false;
}

constexpr std::uint64_t Nv12Size(std::uint32_t width, std::uint32_t height) {
    return static_cast<std::uint64_t>(width) * height * 3 / 2;
}

constexpr std::uint64_t Yuy2Size(std::uint32_t width, std::uint32_t height) {
    return static_cast<std::uint64_t>(width) * height * 2;
}

constexpr std::int32_t SaturatedBitRate(
    std::uint32_t width,
    std::uint32_t height,
    std::uint32_t fps
) {
    const auto bits = static_cast<std::uint64_t>(width) * height * 12 * fps;
    return bits > static_cast<std::uint64_t>(std::numeric_limits<std::int32_t>::max())
        ? std::numeric_limits<std::int32_t>::max()
        : static_cast<std::int32_t>(bits);
}

}  // namespace phonecam
