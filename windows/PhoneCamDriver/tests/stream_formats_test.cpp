#include "stream_formats.h"

#include <cassert>
#include <cstdint>
#include <limits>

int main() {
    using namespace phonecam;
    static_assert(kMediaTypeCount == 15);
    static_assert(kResolutions.front().width == 640);
    static_assert(kResolutions.back().height == 2160);
    static_assert(Nv12Size(3840, 2160) == 12'441'600);
    static_assert(Yuy2Size(3840, 2160) == 16'588'800);
    static_assert(kMaximumNv12Bytes == Nv12Size(3840, 2160));
    static_assert(SaturatedBitRate(3840, 2160, 60) == std::numeric_limits<std::int32_t>::max());
    static_assert(IsSupportedDimensions(2560, 1440));
    static_assert(!IsSupportedDimensions(800, 600));
    static_assert(IsSupportedFrameDuration(666667));
    static_assert(IsSupportedFrameDuration(333333));
    static_assert(IsSupportedFrameDuration(166667));
    static_assert(!IsSupportedFrameDuration(400000));
    assert(kFrameRates[0] == 15 && kFrameRates[2] == 60);
    return 0;
}
