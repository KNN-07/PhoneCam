#include "frame_receiver.h"

#include <algorithm>
#include <array>
#include <chrono>
#include <cstring>
#include <limits>

#if defined(_WIN32)
#include <windows.h>
#endif

namespace {

constexpr std::size_t kFrameHeaderSize = 16;
constexpr std::size_t kReadBufferSize = 64 * 1024;
constexpr std::size_t kMaxPendingBytes = 64 * 1024 * 1024;
constexpr std::size_t kMaxFramePayloadBytes = 100 * 1024 * 1024;

inline std::uint32_t ReadUInt32LE(const std::uint8_t* data) {
    return static_cast<std::uint32_t>(data[0]) | (static_cast<std::uint32_t>(data[1]) << 8) |
           (static_cast<std::uint32_t>(data[2]) << 16) | (static_cast<std::uint32_t>(data[3]) << 24);
}

inline std::uint64_t ReadUInt64LE(const std::uint8_t* data) {
    return static_cast<std::uint64_t>(data[0]) | (static_cast<std::uint64_t>(data[1]) << 8) |
           (static_cast<std::uint64_t>(data[2]) << 16) | (static_cast<std::uint64_t>(data[3]) << 24) |
           (static_cast<std::uint64_t>(data[4]) << 32) | (static_cast<std::uint64_t>(data[5]) << 40) |
           (static_cast<std::uint64_t>(data[6]) << 48) | (static_cast<std::uint64_t>(data[7]) << 56);
}

}

FrameReceiver::FrameReceiver() : running_(false), frame_sequence_(0) {}

FrameReceiver::~FrameReceiver() {
    Stop();
}

HRESULT FrameReceiver::Start() {
    if (running_.exchange(true)) {
        return S_FALSE;
    }

    try {
        receiver_thread_ = std::thread(&FrameReceiver::ReceiverLoop, this);
    } catch (...) {
        running_.store(false);
        return static_cast<HRESULT>(0x80004005L);
    }

    return S_OK;
}

void FrameReceiver::Stop() {
    const bool was_running = running_.exchange(false);
    if (!was_running) {
        return;
    }

    if (receiver_thread_.joinable()) {
        receiver_thread_.join();
    }
}

bool FrameReceiver::WaitForFrame(std::uint64_t* last_sequence, DWORD timeout_ms, PhoneCamFrame* frame) {
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(timeout_ms);
    const std::uint64_t observed_sequence = last_sequence != nullptr ? *last_sequence : 0;

    while (running_.load()) {
        {
            std::lock_guard<std::mutex> lock(mutex_);
            if (frame_sequence_ != 0 && frame_sequence_ != observed_sequence) {
                if (frame != nullptr) {
                    *frame = latest_frame_;
                }

                if (last_sequence != nullptr) {
                    *last_sequence = frame_sequence_;
                }

                return true;
            }
        }

        if (std::chrono::steady_clock::now() >= deadline) {
            break;
        }

        std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }

    return false;
}

bool FrameReceiver::TryGetLatestFrame(PhoneCamFrame* frame) const {
    if (frame == nullptr) {
        return false;
    }

    std::lock_guard<std::mutex> lock(mutex_);
    if (frame_sequence_ == 0) {
        return false;
    }

    *frame = latest_frame_;
    return true;
}

void FrameReceiver::ReceiverLoop() {
#if defined(_WIN32)
    while (running_.load()) {
        HANDLE pipe_handle = CreateNamedPipeW(
            kPipeName,
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            static_cast<DWORD>(kReadBufferSize),
            250,
            nullptr);
        if (pipe_handle == INVALID_HANDLE_VALUE) {
            std::this_thread::sleep_for(std::chrono::milliseconds(250));
            continue;
        }

        bool connected = false;
        while (running_.load() && !connected) {
            if (ConnectNamedPipe(pipe_handle, nullptr)) {
                connected = true;
                break;
            }

            const DWORD connect_error = GetLastError();
            if (connect_error == ERROR_PIPE_CONNECTED) {
                connected = true;
            } else if (connect_error == ERROR_PIPE_LISTENING || connect_error == ERROR_NO_DATA) {
                std::this_thread::sleep_for(std::chrono::milliseconds(10));
            } else {
                break;
            }
        }

        if (connected) {
            ReadFromPipe(pipe_handle);
            DisconnectNamedPipe(pipe_handle);
        }
        CloseHandle(pipe_handle);
    }
#else
    while (running_.load()) {
        std::this_thread::sleep_for(std::chrono::milliseconds(200));
    }
#endif
}

#if defined(_WIN32)
bool FrameReceiver::ReadFromPipe(void* pipe_handle) {
    std::array<std::uint8_t, kReadBufferSize> buffer{};

    while (running_.load()) {
        DWORD bytes_read = 0;
        const BOOL read_ok = ReadFile(
            pipe_handle,
            buffer.data(),
            static_cast<DWORD>(buffer.size()),
            &bytes_read,
            nullptr);

        if (!read_ok) {
            const DWORD read_error = GetLastError();
            if (read_error == ERROR_BROKEN_PIPE || read_error == ERROR_PIPE_NOT_CONNECTED) {
                return true;
            }
            if (read_error == ERROR_NO_DATA) {
                std::this_thread::sleep_for(std::chrono::milliseconds(5));
                continue;
            }

            return false;
        }

        if (bytes_read == 0) {
            return true;
        }

        ProcessBytes(buffer.data(), static_cast<std::size_t>(bytes_read));
    }

    return true;
}
#endif

void FrameReceiver::ProcessBytes(const std::uint8_t* data, std::size_t size) {
    if (data == nullptr || size == 0) {
        return;
    }

    std::unique_lock<std::mutex> lock(mutex_);

    pending_bytes_.insert(pending_bytes_.end(), data, data + size);
    if (pending_bytes_.size() > kMaxPendingBytes) {
        pending_bytes_.clear();
        return;
    }

    while (pending_bytes_.size() >= kFrameHeaderSize) {
        std::uint32_t width = 0;
        std::uint32_t height = 0;
        std::uint64_t timestamp_ns = 0;
        std::size_t payload_size = 0;

        if (!ParsePacketHeader(pending_bytes_, &width, &height, &timestamp_ns, &payload_size)) {
            pending_bytes_.erase(pending_bytes_.begin(), pending_bytes_.begin() + kFrameHeaderSize);
            continue;
        }

        const std::size_t packet_size = kFrameHeaderSize + payload_size;
        if (pending_bytes_.size() < packet_size) {
            return;
        }

        PhoneCamFrame frame;
        frame.width = width;
        frame.height = height;
        frame.timestamp_ns = timestamp_ns;
        frame.payload.assign(pending_bytes_.begin() + kFrameHeaderSize, pending_bytes_.begin() + packet_size);

        latest_frame_ = std::move(frame);
        ++frame_sequence_;

        pending_bytes_.erase(pending_bytes_.begin(), pending_bytes_.begin() + packet_size);
    }
}

bool FrameReceiver::ParsePacketHeader(
    const std::vector<std::uint8_t>& pending,
    std::uint32_t* width,
    std::uint32_t* height,
    std::uint64_t* timestamp_ns,
    std::size_t* payload_size) {
    if (pending.size() < kFrameHeaderSize || width == nullptr || height == nullptr ||
        timestamp_ns == nullptr || payload_size == nullptr) {
        return false;
    }

    const std::uint8_t* header = pending.data();
    const std::uint32_t parsed_width = ReadUInt32LE(header);
    const std::uint32_t parsed_height = ReadUInt32LE(header + 4);
    const std::uint64_t parsed_timestamp = ReadUInt64LE(header + 8);

    if (parsed_width == 0 || parsed_height == 0) {
        return false;
    }

    if ((parsed_width % 2) != 0 || (parsed_height % 2) != 0) {
        return false;
    }

    const std::uint64_t luma_samples =
        static_cast<std::uint64_t>(parsed_width) * static_cast<std::uint64_t>(parsed_height);
    const std::uint64_t total_samples = luma_samples + (luma_samples / 2);

    if (total_samples == 0 || total_samples > kMaxFramePayloadBytes ||
        total_samples > static_cast<std::uint64_t>(std::numeric_limits<std::size_t>::max())) {
        return false;
    }

    *width = parsed_width;
    *height = parsed_height;
    *timestamp_ns = parsed_timestamp;
    *payload_size = static_cast<std::size_t>(total_samples);
    return true;
}
