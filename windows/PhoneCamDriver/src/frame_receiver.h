#pragma once

#include <array>
#include <atomic>
#include <cstdint>
#include <mutex>
#include <thread>
#include <vector>

#if defined(_WIN32)
#include <windows.h>
#else
using HRESULT = long;
constexpr HRESULT S_OK = 0;
constexpr HRESULT S_FALSE = 1;
#endif

struct PhoneCamFrame {
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    std::uint64_t timestamp_ns = 0;
    std::vector<std::uint8_t> payload;
};

class FrameReceiver {
  public:
    FrameReceiver();
    ~FrameReceiver();

    HRESULT Start();
    void Stop();

    bool TryGetLatestFrame(PhoneCamFrame* frame) const;
    void PublishFormat(std::uint32_t width, std::uint32_t height, std::uint8_t fps);

  private:
    void ReceiverLoop();
#if defined(_WIN32)
    bool ReadFromPipe(void* pipe_handle);
#endif
    void ProcessBytes(const std::uint8_t* data, std::size_t size);

    static bool ParsePacketHeader(
        const std::vector<std::uint8_t>& pending,
        std::uint32_t* width,
        std::uint32_t* height,
        std::uint64_t* timestamp_ns,
        std::size_t* payload_size);

    static inline constexpr wchar_t kPipeName[] = LR"(\\.\pipe\PhoneCam)";

    std::atomic<bool> running_;
    std::thread receiver_thread_;

    mutable std::mutex pipe_mutex_;
    void* pipe_handle_ = nullptr;
    std::array<std::uint8_t, 16> latest_format_event_{};
    bool has_latest_format_event_ = false;
    mutable std::mutex mutex_;
    std::vector<std::uint8_t> pending_bytes_;

    PhoneCamFrame latest_frame_;
    std::uint64_t frame_sequence_;
};
