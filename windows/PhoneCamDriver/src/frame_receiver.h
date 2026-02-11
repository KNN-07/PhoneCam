#pragma once

#include <atomic>
#include <cstdint>
#include <mutex>
#include <thread>
#include <vector>

#if defined(_WIN32)
#include <windows.h>
#else
using DWORD = std::uint32_t;
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

    bool WaitForFrame(std::uint64_t* last_sequence, DWORD timeout_ms, PhoneCamFrame* frame);
    bool TryGetLatestFrame(PhoneCamFrame* frame) const;

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

    mutable std::mutex mutex_;
    std::vector<std::uint8_t> pending_bytes_;

    PhoneCamFrame latest_frame_;
    std::uint64_t frame_sequence_;
};
