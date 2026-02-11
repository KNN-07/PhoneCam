#include "output_pin.h"

#if defined(_WIN32)

#include "filter.h"
#include "frame_receiver.h"
#include "guids.h"

#include <algorithm>
#include <array>
#include <atomic>
#include <cstring>
#include <limits>
#include <new>

#include <ksmedia.h>

#ifndef KSPROPERTY_SUPPORT_GET
#define KSPROPERTY_SUPPORT_GET 0x00000001
#endif

namespace {

constexpr REFERENCE_TIME kDefaultFrameDuration = 333333;
constexpr ULONG kSupportedMediaTypeCount = 2;

const GUID kMediaSubtypeYuyv = {
    MAKEFOURCC('Y', 'U', 'Y', 'V'),
    0x0000,
    0x0010,
    {0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71}};

void ResetMediaType(AM_MEDIA_TYPE* media_type) {
    if (media_type != nullptr) {
        ZeroMemory(media_type, sizeof(*media_type));
    }
}

void ReleaseMediaType(AM_MEDIA_TYPE* media_type) {
    if (media_type == nullptr) {
        return;
    }

    if (media_type->cbFormat != 0 && media_type->pbFormat != nullptr) {
        CoTaskMemFree(media_type->pbFormat);
        media_type->pbFormat = nullptr;
        media_type->cbFormat = 0;
    }

    if (media_type->pUnk != nullptr) {
        media_type->pUnk->Release();
        media_type->pUnk = nullptr;
    }
}

HRESULT CloneMediaType(AM_MEDIA_TYPE* destination, const AM_MEDIA_TYPE* source) {
    if (destination == nullptr || source == nullptr) {
        return E_POINTER;
    }

    ResetMediaType(destination);
    *destination = *source;
    destination->pUnk = nullptr;
    destination->pbFormat = nullptr;

    if (source->cbFormat != 0 && source->pbFormat != nullptr) {
        destination->pbFormat = static_cast<BYTE*>(CoTaskMemAlloc(source->cbFormat));
        if (destination->pbFormat == nullptr) {
            ResetMediaType(destination);
            return E_OUTOFMEMORY;
        }

        CopyMemory(destination->pbFormat, source->pbFormat, source->cbFormat);
    } else {
        destination->cbFormat = 0;
    }

    if (source->pUnk != nullptr) {
        source->pUnk->AddRef();
        destination->pUnk = source->pUnk;
    }

    return S_OK;
}

class MediaTypeEnum final : public IEnumMediaTypes {
  public:
    static HRESULT Create(const AM_MEDIA_TYPE* media_types, ULONG count, IEnumMediaTypes** result) {
        if (result == nullptr) {
            return E_POINTER;
        }

        *result = nullptr;

        MediaTypeEnum* enumerator = new (std::nothrow) MediaTypeEnum();
        if (enumerator == nullptr) {
            return E_OUTOFMEMORY;
        }

        for (ULONG i = 0; i < count && i < kSupportedMediaTypeCount; ++i) {
            const HRESULT hr = CloneMediaType(&enumerator->media_types_[i], &media_types[i]);
            if (FAILED(hr)) {
                enumerator->Release();
                return hr;
            }

            ++enumerator->count_;
        }

        *result = enumerator;
        return S_OK;
    }

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
        if (ppv == nullptr) {
            return E_POINTER;
        }

        if (riid == IID_IUnknown || riid == IID_IEnumMediaTypes) {
            *ppv = static_cast<IEnumMediaTypes*>(this);
            AddRef();
            return S_OK;
        }

        *ppv = nullptr;
        return E_NOINTERFACE;
    }

    STDMETHODIMP_(ULONG) AddRef() override {
        return static_cast<ULONG>(InterlockedIncrement(&ref_count_));
    }

    STDMETHODIMP_(ULONG) Release() override {
        const ULONG count = static_cast<ULONG>(InterlockedDecrement(&ref_count_));
        if (count == 0) {
            delete this;
        }

        return count;
    }

    STDMETHODIMP Next(ULONG cMediaTypes, AM_MEDIA_TYPE** ppMediaTypes, ULONG* pcFetched) override {
        if (ppMediaTypes == nullptr) {
            return E_POINTER;
        }

        if (cMediaTypes > 1 && pcFetched == nullptr) {
            return E_POINTER;
        }

        ULONG fetched = 0;
        while (fetched < cMediaTypes && index_ < count_) {
            AM_MEDIA_TYPE* cloned = static_cast<AM_MEDIA_TYPE*>(CoTaskMemAlloc(sizeof(AM_MEDIA_TYPE)));
            if (cloned == nullptr) {
                break;
            }

            const HRESULT hr = CloneMediaType(cloned, &media_types_[index_]);
            if (FAILED(hr)) {
                CoTaskMemFree(cloned);
                break;
            }

            ppMediaTypes[fetched] = cloned;
            ++fetched;
            ++index_;
        }

        if (pcFetched != nullptr) {
            *pcFetched = fetched;
        }

        return fetched == cMediaTypes ? S_OK : S_FALSE;
    }

    STDMETHODIMP Skip(ULONG cMediaTypes) override {
        const ULONG remaining = count_ - index_;
        const ULONG skipped = std::min(cMediaTypes, remaining);
        index_ += skipped;
        return skipped == cMediaTypes ? S_OK : S_FALSE;
    }

    STDMETHODIMP Reset() override {
        index_ = 0;
        return S_OK;
    }

    STDMETHODIMP Clone(IEnumMediaTypes** ppEnum) override {
        if (ppEnum == nullptr) {
            return E_POINTER;
        }

        *ppEnum = nullptr;

        IEnumMediaTypes* cloned = nullptr;
        const HRESULT hr = Create(media_types_.data(), count_, &cloned);
        if (FAILED(hr)) {
            return hr;
        }

        MediaTypeEnum* typed = static_cast<MediaTypeEnum*>(cloned);
        typed->index_ = index_;
        *ppEnum = cloned;
        return S_OK;
    }

  private:
    MediaTypeEnum() : ref_count_(1), count_(0), index_(0) {
        for (auto& media_type : media_types_) {
            ResetMediaType(&media_type);
        }
    }

    ~MediaTypeEnum() override {
        for (ULONG i = 0; i < count_; ++i) {
            ReleaseMediaType(&media_types_[i]);
        }
    }

    volatile long ref_count_;
    std::array<AM_MEDIA_TYPE, kSupportedMediaTypeCount> media_types_;
    ULONG count_;
    ULONG index_;
};

LONG MakeImageSizeBytes(LONG width, LONG height, const GUID& subtype) {
    if (width <= 0 || height <= 0) {
        return 0;
    }

    const std::int64_t samples = static_cast<std::int64_t>(width) * static_cast<std::int64_t>(height);
    if (samples <= 0) {
        return 0;
    }

    std::int64_t bytes = 0;
    if (subtype == MEDIASUBTYPE_NV12) {
        bytes = samples + (samples / 2);
    } else {
        bytes = samples * 2;
    }

    if (bytes <= 0 || bytes > std::numeric_limits<LONG>::max()) {
        return 0;
    }

    return static_cast<LONG>(bytes);
}

} 

PhoneCamOutputPin::PhoneCamOutputPin(PhoneCamFilter* filter)
    : ref_count_(1),
      filter_(filter),
      connected_pin_(nullptr),
      mem_input_pin_(nullptr),
      allocator_(nullptr),
      has_connected_media_type_(false),
      current_format_{1280, 720, MEDIASUBTYPE_NV12, kDefaultFrameDuration},
      streaming_(false),
      stop_requested_(false),
      last_frame_sequence_(0),
      has_last_frame_(false),
      stream_start_(0),
      frame_index_(0) {
    ResetMediaType(&connected_media_type_);
    if (filter_ != nullptr) {
        filter_->AddRef();
    }
}

PhoneCamOutputPin::~PhoneCamOutputPin() {
    StopStreaming();

    std::lock_guard<std::mutex> guard(lock_);
    ReleaseAllocatorLocked();

    if (mem_input_pin_ != nullptr) {
        mem_input_pin_->Release();
        mem_input_pin_ = nullptr;
    }

    if (connected_pin_ != nullptr) {
        connected_pin_->Release();
        connected_pin_ = nullptr;
    }

    if (has_connected_media_type_) {
        FreeMediaType(&connected_media_type_);
        has_connected_media_type_ = false;
    }

    if (filter_ != nullptr) {
        filter_->Release();
        filter_ = nullptr;
    }
}

HRESULT PhoneCamOutputPin::StartStreaming(REFERENCE_TIME stream_start) {
    {
        std::lock_guard<std::mutex> guard(lock_);
        if (streaming_.load(std::memory_order_relaxed)) {
            return S_OK;
        }

        if (connected_pin_ == nullptr || mem_input_pin_ == nullptr) {
            return VFW_E_NOT_CONNECTED;
        }

        const HRESULT allocator_hr = EnsureAllocatorLocked();
        if (FAILED(allocator_hr)) {
            return allocator_hr;
        }

        stream_start_ = stream_start;
        frame_index_ = 0;
        last_frame_sequence_ = 0;
        has_last_frame_ = false;
        stop_requested_.store(false, std::memory_order_release);
        streaming_.store(true, std::memory_order_release);
    }

    const HRESULT receiver_hr = frame_receiver_.Start();
    if (FAILED(receiver_hr) && receiver_hr != S_FALSE) {
        streaming_.store(false, std::memory_order_release);
        return receiver_hr;
    }

    try {
        streaming_thread_ = std::thread(&PhoneCamOutputPin::StreamingLoop, this);
    } catch (...) {
        stop_requested_.store(true, std::memory_order_release);
        streaming_.store(false, std::memory_order_release);
        frame_receiver_.Stop();
        return E_FAIL;
    }

    return S_OK;
}

HRESULT PhoneCamOutputPin::PauseStreaming() {
    return StopStreaming();
}

HRESULT PhoneCamOutputPin::StopStreaming() {
    bool was_streaming = false;
    {
        std::lock_guard<std::mutex> guard(lock_);
        was_streaming = streaming_.load(std::memory_order_relaxed);
        stop_requested_.store(true, std::memory_order_release);
    }

    if (streaming_thread_.joinable()) {
        streaming_thread_.join();
    }

    frame_receiver_.Stop();

    {
        std::lock_guard<std::mutex> guard(lock_);
        streaming_.store(false, std::memory_order_release);
    }

    return was_streaming ? S_OK : S_FALSE;
}

bool PhoneCamOutputPin::IsConnected() const {
    std::lock_guard<std::mutex> guard(lock_);
    return connected_pin_ != nullptr;
}

STDMETHODIMP PhoneCamOutputPin::QueryInterface(REFIID riid, void** ppv) {
    if (ppv == nullptr) {
        return E_POINTER;
    }

    if (riid == IID_IUnknown || riid == IID_IPin) {
        *ppv = static_cast<IPin*>(this);
    } else if (riid == IID_IAMStreamConfig) {
        *ppv = static_cast<IAMStreamConfig*>(this);
    } else if (riid == IID_IKsPropertySet) {
        *ppv = static_cast<IKsPropertySet*>(this);
    } else if (riid == IID_IMemInputPin) {
        *ppv = static_cast<IMemInputPin*>(this);
    } else {
        *ppv = nullptr;
        return E_NOINTERFACE;
    }

    AddRef();
    return S_OK;
}

STDMETHODIMP_(ULONG) PhoneCamOutputPin::AddRef() {
    return ref_count_.fetch_add(1, std::memory_order_relaxed) + 1;
}

STDMETHODIMP_(ULONG) PhoneCamOutputPin::Release() {
    const ULONG count = ref_count_.fetch_sub(1, std::memory_order_acq_rel) - 1;
    if (count == 0) {
        delete this;
    }

    return count;
}

STDMETHODIMP PhoneCamOutputPin::Connect(IPin* pReceivePin, const AM_MEDIA_TYPE* pmt) {
    if (pReceivePin == nullptr) {
        return E_POINTER;
    }

    {
        std::lock_guard<std::mutex> guard(lock_);
        if (connected_pin_ != nullptr) {
            return VFW_E_ALREADY_CONNECTED;
        }
    }

    AM_MEDIA_TYPE proposed;
    ResetMediaType(&proposed);

    HRESULT hr = S_OK;
    if (pmt != nullptr) {
        if (!IsSupportedMediaType(pmt)) {
            return VFW_E_TYPE_NOT_ACCEPTED;
        }

        hr = CopyMediaType(&proposed, pmt);
    } else {
        hr = BuildMediaType(current_format_, &proposed);
    }

    if (FAILED(hr)) {
        return hr;
    }

    hr = pReceivePin->ReceiveConnection(this, &proposed);
    if (SUCCEEDED(hr)) {
        std::lock_guard<std::mutex> guard(lock_);
        hr = CompleteConnectionLocked(pReceivePin, &proposed);
        if (FAILED(hr)) {
            pReceivePin->Disconnect();
        }
    }

    FreeMediaType(&proposed);
    return hr;
}

STDMETHODIMP PhoneCamOutputPin::ReceiveConnection(IPin* pConnector, const AM_MEDIA_TYPE* pmt) {
    if (pConnector == nullptr || pmt == nullptr) {
        return E_POINTER;
    }

    if (!IsSupportedMediaType(pmt)) {
        return VFW_E_TYPE_NOT_ACCEPTED;
    }

    std::lock_guard<std::mutex> guard(lock_);
    if (connected_pin_ != nullptr) {
        return VFW_E_ALREADY_CONNECTED;
    }

    return CompleteConnectionLocked(pConnector, pmt);
}

STDMETHODIMP PhoneCamOutputPin::Disconnect() {
    StopStreaming();

    IPin* peer = nullptr;
    {
        std::lock_guard<std::mutex> guard(lock_);
        if (connected_pin_ == nullptr) {
            return S_FALSE;
        }

        peer = connected_pin_;
        peer->AddRef();

        ReleaseAllocatorLocked();

        if (mem_input_pin_ != nullptr) {
            mem_input_pin_->Release();
            mem_input_pin_ = nullptr;
        }

        connected_pin_->Release();
        connected_pin_ = nullptr;

        if (has_connected_media_type_) {
            FreeMediaType(&connected_media_type_);
            has_connected_media_type_ = false;
        }
    }

    peer->Disconnect();
    peer->Release();
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::ConnectedTo(IPin** ppPin) {
    if (ppPin == nullptr) {
        return E_POINTER;
    }

    *ppPin = nullptr;

    std::lock_guard<std::mutex> guard(lock_);
    if (connected_pin_ == nullptr) {
        return VFW_E_NOT_CONNECTED;
    }

    connected_pin_->AddRef();
    *ppPin = connected_pin_;
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::ConnectionMediaType(AM_MEDIA_TYPE* pmt) {
    if (pmt == nullptr) {
        return E_POINTER;
    }

    std::lock_guard<std::mutex> guard(lock_);
    if (!has_connected_media_type_) {
        return VFW_E_NOT_CONNECTED;
    }

    return CopyMediaType(pmt, &connected_media_type_);
}

STDMETHODIMP PhoneCamOutputPin::QueryPinInfo(PIN_INFO* pInfo) {
    if (pInfo == nullptr) {
        return E_POINTER;
    }

    ZeroMemory(pInfo, sizeof(*pInfo));
    pInfo->dir = PINDIR_OUTPUT;
    lstrcpynW(pInfo->achName, kPhoneCamOutputPinName, static_cast<int>(sizeof(pInfo->achName) / sizeof(pInfo->achName[0])));

    if (filter_ != nullptr) {
        filter_->AddRef();
        pInfo->pFilter = filter_;
    }

    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::QueryDirection(PIN_DIRECTION* pPinDir) {
    if (pPinDir == nullptr) {
        return E_POINTER;
    }

    *pPinDir = PINDIR_OUTPUT;
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::QueryId(LPWSTR* Id) {
    if (Id == nullptr) {
        return E_POINTER;
    }

    *Id = nullptr;

    const UINT char_count = lstrlenW(kPhoneCamOutputPinName) + 1;
    const SIZE_T bytes = static_cast<SIZE_T>(char_count) * sizeof(WCHAR);
    LPWSTR buffer = static_cast<LPWSTR>(CoTaskMemAlloc(bytes));
    if (buffer == nullptr) {
        return E_OUTOFMEMORY;
    }

    CopyMemory(buffer, kPhoneCamOutputPinName, bytes);
    *Id = buffer;
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::QueryAccept(const AM_MEDIA_TYPE* pmt) {
    return IsSupportedMediaType(pmt) ? S_OK : S_FALSE;
}

STDMETHODIMP PhoneCamOutputPin::EnumMediaTypes(IEnumMediaTypes** ppEnum) {
    if (ppEnum == nullptr) {
        return E_POINTER;
    }

    *ppEnum = nullptr;

    AM_MEDIA_TYPE media_types[kSupportedMediaTypeCount];
    for (auto& media_type : media_types) {
        ResetMediaType(&media_type);
    }

    ULONG built_count = 0;
    for (ULONG i = 0; i < kSupportedMediaTypeCount; ++i) {
        const HRESULT hr = BuildMediaTypeForIndex(i, &media_types[i]);
        if (FAILED(hr)) {
            for (ULONG j = 0; j < built_count; ++j) {
                FreeMediaType(&media_types[j]);
            }
            return hr;
        }

        ++built_count;
    }

    const HRESULT create_hr = MediaTypeEnum::Create(media_types, built_count, ppEnum);
    for (ULONG i = 0; i < built_count; ++i) {
        FreeMediaType(&media_types[i]);
    }

    return create_hr;
}

STDMETHODIMP PhoneCamOutputPin::QueryInternalConnections(IPin**, ULONG*) {
    return E_NOTIMPL;
}

STDMETHODIMP PhoneCamOutputPin::EndOfStream() {
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::BeginFlush() {
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::EndFlush() {
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::NewSegment(REFERENCE_TIME, REFERENCE_TIME, double) {
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::SetFormat(AM_MEDIA_TYPE* pmt) {
    if (pmt == nullptr) {
        return E_POINTER;
    }

    if (!IsSupportedMediaType(pmt)) {
        return VFW_E_INVALIDMEDIATYPE;
    }

    const VIDEOINFOHEADER2* video_info = reinterpret_cast<const VIDEOINFOHEADER2*>(pmt->pbFormat);
    StreamFormat requested = {
        static_cast<LONG>(video_info->bmiHeader.biWidth),
        static_cast<LONG>(video_info->bmiHeader.biHeight > 0 ? video_info->bmiHeader.biHeight : -video_info->bmiHeader.biHeight),
        pmt->subtype,
        video_info->AvgTimePerFrame > 0 ? video_info->AvgTimePerFrame : kDefaultFrameDuration};

    std::lock_guard<std::mutex> guard(lock_);
    current_format_ = requested;

    if (has_connected_media_type_) {
        FreeMediaType(&connected_media_type_);
        has_connected_media_type_ = false;
    }

    if (connected_pin_ != nullptr) {
        const HRESULT copy_hr = CopyMediaType(&connected_media_type_, pmt);
        if (FAILED(copy_hr)) {
            return copy_hr;
        }
        has_connected_media_type_ = true;

        ReleaseAllocatorLocked();
        return EnsureAllocatorLocked();
    }

    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::GetFormat(AM_MEDIA_TYPE** ppmt) {
    if (ppmt == nullptr) {
        return E_POINTER;
    }

    *ppmt = nullptr;
    AM_MEDIA_TYPE* media_type = static_cast<AM_MEDIA_TYPE*>(CoTaskMemAlloc(sizeof(AM_MEDIA_TYPE)));
    if (media_type == nullptr) {
        return E_OUTOFMEMORY;
    }

    StreamFormat format{};
    {
        std::lock_guard<std::mutex> guard(lock_);
        format = current_format_;
    }

    const HRESULT hr = BuildMediaType(format, media_type);
    if (FAILED(hr)) {
        CoTaskMemFree(media_type);
        return hr;
    }

    *ppmt = media_type;
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::GetNumberOfCapabilities(int* piCount, int* piSize) {
    if (piCount == nullptr || piSize == nullptr) {
        return E_POINTER;
    }

    *piCount = static_cast<int>(kSupportedMediaTypeCount);
    *piSize = sizeof(VIDEO_STREAM_CONFIG_CAPS);
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::GetStreamCaps(int iIndex, AM_MEDIA_TYPE** ppmt, BYTE* pSCC) {
    if (ppmt == nullptr || pSCC == nullptr) {
        return E_POINTER;
    }

    *ppmt = nullptr;
    if (iIndex < 0 || iIndex >= static_cast<int>(kSupportedMediaTypeCount)) {
        return S_FALSE;
    }

    AM_MEDIA_TYPE* media_type = static_cast<AM_MEDIA_TYPE*>(CoTaskMemAlloc(sizeof(AM_MEDIA_TYPE)));
    if (media_type == nullptr) {
        return E_OUTOFMEMORY;
    }

    const HRESULT build_hr = BuildMediaTypeForIndex(static_cast<ULONG>(iIndex), media_type);
    if (FAILED(build_hr)) {
        CoTaskMemFree(media_type);
        return build_hr;
    }

    const VIDEOINFOHEADER2* video_info = reinterpret_cast<const VIDEOINFOHEADER2*>(media_type->pbFormat);
    const LONG width = video_info->bmiHeader.biWidth;
    const LONG height = video_info->bmiHeader.biHeight > 0 ? video_info->bmiHeader.biHeight : -video_info->bmiHeader.biHeight;

    VIDEO_STREAM_CONFIG_CAPS* caps = reinterpret_cast<VIDEO_STREAM_CONFIG_CAPS*>(pSCC);
    ZeroMemory(caps, sizeof(VIDEO_STREAM_CONFIG_CAPS));
    caps->guid = FORMAT_VideoInfo2;
    caps->VideoStandard = AnalogVideo_None;
    caps->InputSize.cx = width;
    caps->InputSize.cy = height;
    caps->MinCroppingSize.cx = width;
    caps->MinCroppingSize.cy = height;
    caps->MaxCroppingSize.cx = width;
    caps->MaxCroppingSize.cy = height;
    caps->CropGranularityX = 1;
    caps->CropGranularityY = 1;
    caps->CropAlignX = 1;
    caps->CropAlignY = 1;
    caps->MinOutputSize.cx = width;
    caps->MinOutputSize.cy = height;
    caps->MaxOutputSize.cx = width;
    caps->MaxOutputSize.cy = height;
    caps->OutputGranularityX = 1;
    caps->OutputGranularityY = 1;
    caps->StretchTapsX = 0;
    caps->StretchTapsY = 0;
    caps->ShrinkTapsX = 0;
    caps->ShrinkTapsY = 0;
    caps->MinFrameInterval = kDefaultFrameDuration;
    caps->MaxFrameInterval = kDefaultFrameDuration;

    const std::int64_t bitrate = static_cast<std::int64_t>(width) * static_cast<std::int64_t>(height) * 12 * 30;
    const LONG safe_bitrate =
        bitrate > std::numeric_limits<LONG>::max() ? std::numeric_limits<LONG>::max() : static_cast<LONG>(bitrate);
    caps->MinBitsPerSecond = safe_bitrate;
    caps->MaxBitsPerSecond = safe_bitrate;

    *ppmt = media_type;
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::Set(REFGUID, DWORD, void*, DWORD, void*, DWORD) {
    return E_NOTIMPL;
}

STDMETHODIMP PhoneCamOutputPin::Get(
    REFGUID guidPropSet,
    DWORD dwPropID,
    void*,
    DWORD,
    void* pPropData,
    DWORD cbPropData,
    DWORD* pcbReturned) {
    if (pcbReturned != nullptr) {
        *pcbReturned = 0;
    }

    if (guidPropSet != AMPROPSETID_Pin || dwPropID != AMPROPERTY_PIN_CATEGORY) {
        return E_NOTIMPL;
    }

    if (pPropData == nullptr || cbPropData < sizeof(GUID)) {
        return E_POINTER;
    }

    *reinterpret_cast<GUID*>(pPropData) = PIN_CATEGORY_CAPTURE;
    if (pcbReturned != nullptr) {
        *pcbReturned = sizeof(GUID);
    }

    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::QuerySupported(REFGUID guidPropSet, DWORD dwPropID, DWORD* pTypeSupport) {
    if (pTypeSupport == nullptr) {
        return E_POINTER;
    }

    *pTypeSupport = 0;

    if (guidPropSet == AMPROPSETID_Pin && dwPropID == AMPROPERTY_PIN_CATEGORY) {
        *pTypeSupport = KSPROPERTY_SUPPORT_GET;
        return S_OK;
    }

    return S_FALSE;
}

STDMETHODIMP PhoneCamOutputPin::GetAllocator(IMemAllocator** ppAllocator) {
    if (ppAllocator == nullptr) {
        return E_POINTER;
    }

    *ppAllocator = nullptr;

    std::lock_guard<std::mutex> guard(lock_);
    if (allocator_ == nullptr) {
        return VFW_E_NO_ALLOCATOR;
    }

    allocator_->AddRef();
    *ppAllocator = allocator_;
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::NotifyAllocator(IMemAllocator* pAllocator, BOOL) {
    if (pAllocator == nullptr) {
        return E_POINTER;
    }

    std::lock_guard<std::mutex> guard(lock_);
    ReleaseAllocatorLocked();
    allocator_ = pAllocator;
    allocator_->AddRef();
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::GetAllocatorRequirements(ALLOCATOR_PROPERTIES* pProps) {
    if (pProps == nullptr) {
        return E_POINTER;
    }

    std::lock_guard<std::mutex> guard(lock_);
    pProps->cbBuffer = FrameBufferSizeLocked();
    pProps->cBuffers = 4;
    pProps->cbAlign = 1;
    pProps->cbPrefix = 0;
    return S_OK;
}

STDMETHODIMP PhoneCamOutputPin::Receive(IMediaSample*) {
    return E_UNEXPECTED;
}

STDMETHODIMP PhoneCamOutputPin::ReceiveMultiple(IMediaSample**, long, long* nSamplesProcessed) {
    if (nSamplesProcessed != nullptr) {
        *nSamplesProcessed = 0;
    }

    return E_UNEXPECTED;
}

STDMETHODIMP PhoneCamOutputPin::ReceiveCanBlock() {
    return S_FALSE;
}

HRESULT PhoneCamOutputPin::CompleteConnectionLocked(IPin* peer_pin, const AM_MEDIA_TYPE* media_type) {
    if (peer_pin == nullptr || media_type == nullptr) {
        return E_POINTER;
    }

    if (connected_pin_ != nullptr) {
        return VFW_E_ALREADY_CONNECTED;
    }

    IMemInputPin* mem_input = nullptr;
    HRESULT hr = peer_pin->QueryInterface(IID_IMemInputPin, reinterpret_cast<void**>(&mem_input));
    if (FAILED(hr) || mem_input == nullptr) {
        return VFW_E_NO_TRANSPORT;
    }

    AM_MEDIA_TYPE copied;
    ResetMediaType(&copied);
    hr = CopyMediaType(&copied, media_type);
    if (FAILED(hr)) {
        mem_input->Release();
        return hr;
    }

    connected_pin_ = peer_pin;
    connected_pin_->AddRef();
    mem_input_pin_ = mem_input;

    if (has_connected_media_type_) {
        FreeMediaType(&connected_media_type_);
        has_connected_media_type_ = false;
    }

    connected_media_type_ = copied;
    has_connected_media_type_ = true;

    const VIDEOINFOHEADER2* video_info = reinterpret_cast<const VIDEOINFOHEADER2*>(media_type->pbFormat);
    current_format_.width = video_info->bmiHeader.biWidth;
    current_format_.height = video_info->bmiHeader.biHeight > 0 ? video_info->bmiHeader.biHeight : -video_info->bmiHeader.biHeight;
    current_format_.subtype = media_type->subtype;
    current_format_.avg_time_per_frame =
        video_info->AvgTimePerFrame > 0 ? video_info->AvgTimePerFrame : kDefaultFrameDuration;

    hr = EnsureAllocatorLocked();
    if (FAILED(hr)) {
        ReleaseAllocatorLocked();

        if (mem_input_pin_ != nullptr) {
            mem_input_pin_->Release();
            mem_input_pin_ = nullptr;
        }

        if (connected_pin_ != nullptr) {
            connected_pin_->Release();
            connected_pin_ = nullptr;
        }

        if (has_connected_media_type_) {
            FreeMediaType(&connected_media_type_);
            has_connected_media_type_ = false;
        }
    }

    return hr;
}

HRESULT PhoneCamOutputPin::EnsureAllocatorLocked() {
    if (mem_input_pin_ == nullptr) {
        return VFW_E_NO_TRANSPORT;
    }

    if (allocator_ != nullptr) {
        return S_OK;
    }

    IMemAllocator* allocator = nullptr;
    HRESULT hr = mem_input_pin_->GetAllocator(&allocator);
    if (FAILED(hr) || allocator == nullptr) {
        hr = CoCreateInstance(
            CLSID_MemoryAllocator,
            nullptr,
            CLSCTX_INPROC_SERVER,
            IID_IMemAllocator,
            reinterpret_cast<void**>(&allocator));
        if (FAILED(hr) || allocator == nullptr) {
            return hr;
        }
    }

    ALLOCATOR_PROPERTIES requested = {};
    requested.cBuffers = 4;
    requested.cbBuffer = FrameBufferSizeLocked();
    requested.cbAlign = 1;
    requested.cbPrefix = 0;

    ALLOCATOR_PROPERTIES actual = {};
    hr = allocator->SetProperties(&requested, &actual);
    if (FAILED(hr)) {
        allocator->Release();
        return hr;
    }

    hr = mem_input_pin_->NotifyAllocator(allocator, FALSE);
    if (FAILED(hr)) {
        allocator->Release();
        return hr;
    }

    hr = allocator->Commit();
    if (FAILED(hr)) {
        allocator->Release();
        return hr;
    }

    allocator_ = allocator;
    return S_OK;
}

void PhoneCamOutputPin::ReleaseAllocatorLocked() {
    if (allocator_ == nullptr) {
        return;
    }

    allocator_->Decommit();
    allocator_->Release();
    allocator_ = nullptr;
}

bool PhoneCamOutputPin::IsSupportedSubtype(REFGUID subtype) const {
    return subtype == MEDIASUBTYPE_NV12 || subtype == MEDIASUBTYPE_YUY2 || subtype == kMediaSubtypeYuyv;
}

bool PhoneCamOutputPin::IsSupportedMediaType(const AM_MEDIA_TYPE* media_type) const {
    if (media_type == nullptr) {
        return false;
    }

    if (media_type->majortype != MEDIATYPE_Video) {
        return false;
    }

    if (!IsSupportedSubtype(media_type->subtype)) {
        return false;
    }

    if (media_type->formattype != FORMAT_VideoInfo2 || media_type->pbFormat == nullptr ||
        media_type->cbFormat < sizeof(VIDEOINFOHEADER2)) {
        return false;
    }

    const VIDEOINFOHEADER2* video_info = reinterpret_cast<const VIDEOINFOHEADER2*>(media_type->pbFormat);
    const LONG width = video_info->bmiHeader.biWidth;
    const LONG height = video_info->bmiHeader.biHeight > 0 ? video_info->bmiHeader.biHeight : -video_info->bmiHeader.biHeight;

    if (width <= 0 || height <= 0) {
        return false;
    }

    if (media_type->subtype == MEDIASUBTYPE_NV12 && ((width % 2) != 0 || (height % 2) != 0)) {
        return false;
    }

    return true;
}

HRESULT PhoneCamOutputPin::BuildMediaType(const StreamFormat& format, AM_MEDIA_TYPE* media_type) const {
    if (media_type == nullptr) {
        return E_POINTER;
    }

    ResetMediaType(media_type);

    if (format.width <= 0 || format.height <= 0 || !IsSupportedSubtype(format.subtype)) {
        return E_INVALIDARG;
    }

    VIDEOINFOHEADER2* video_info = static_cast<VIDEOINFOHEADER2*>(CoTaskMemAlloc(sizeof(VIDEOINFOHEADER2)));
    if (video_info == nullptr) {
        return E_OUTOFMEMORY;
    }

    ZeroMemory(video_info, sizeof(VIDEOINFOHEADER2));
    SetRect(&video_info->rcSource, 0, 0, format.width, format.height);
    SetRect(&video_info->rcTarget, 0, 0, format.width, format.height);
    video_info->dwPictAspectRatioX = static_cast<DWORD>(format.width);
    video_info->dwPictAspectRatioY = static_cast<DWORD>(format.height);
    video_info->AvgTimePerFrame = format.avg_time_per_frame > 0 ? format.avg_time_per_frame : kDefaultFrameDuration;
    video_info->bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    video_info->bmiHeader.biWidth = format.width;
    video_info->bmiHeader.biHeight = format.height;
    video_info->bmiHeader.biPlanes = 1;
    video_info->bmiHeader.biCompression =
        format.subtype == MEDIASUBTYPE_NV12 ? MAKEFOURCC('N', 'V', '1', '2') : MAKEFOURCC('Y', 'U', 'Y', '2');
    video_info->bmiHeader.biBitCount = format.subtype == MEDIASUBTYPE_NV12 ? 12 : 16;
    video_info->bmiHeader.biSizeImage = MakeImageSizeBytes(format.width, format.height, format.subtype);

    media_type->majortype = MEDIATYPE_Video;
    media_type->subtype = format.subtype;
    media_type->bFixedSizeSamples = TRUE;
    media_type->bTemporalCompression = FALSE;
    media_type->lSampleSize = video_info->bmiHeader.biSizeImage;
    media_type->formattype = FORMAT_VideoInfo2;
    media_type->pUnk = nullptr;
    media_type->cbFormat = sizeof(VIDEOINFOHEADER2);
    media_type->pbFormat = reinterpret_cast<BYTE*>(video_info);
    return S_OK;
}

HRESULT PhoneCamOutputPin::BuildMediaTypeForIndex(ULONG index, AM_MEDIA_TYPE* media_type) const {
    if (index >= kSupportedMediaTypeCount) {
        return S_FALSE;
    }

    StreamFormat format{};
    if (index == 0) {
        format = {1280, 720, MEDIASUBTYPE_NV12, kDefaultFrameDuration};
    } else {
        format = {1920, 1080, MEDIASUBTYPE_NV12, kDefaultFrameDuration};
    }

    return BuildMediaType(format, media_type);
}

LONG PhoneCamOutputPin::FrameBufferSizeLocked() const {
    return FrameBufferSizeForFormat(current_format_);
}

LONG PhoneCamOutputPin::FrameBufferSizeForFormat(const StreamFormat& format) {
    return MakeImageSizeBytes(format.width, format.height, format.subtype);
}

REFERENCE_TIME PhoneCamOutputPin::FrameDurationLocked() const {
    return current_format_.avg_time_per_frame > 0 ? current_format_.avg_time_per_frame : kDefaultFrameDuration;
}

void PhoneCamOutputPin::StreamingLoop() {
    std::uint64_t sequence = 0;

    while (!stop_requested_.load(std::memory_order_acquire)) {
        PhoneCamFrame frame;
        if (!frame_receiver_.WaitForFrame(&sequence, 100, &frame)) {
            continue;
        }

        IMemAllocator* allocator = nullptr;
        IMemInputPin* sink = nullptr;
        REFERENCE_TIME start = 0;
        REFERENCE_TIME stop = 0;

        {
            std::lock_guard<std::mutex> guard(lock_);
            if (!streaming_.load(std::memory_order_relaxed) || allocator_ == nullptr || mem_input_pin_ == nullptr) {
                continue;
            }

            allocator = allocator_;
            sink = mem_input_pin_;
            allocator->AddRef();
            sink->AddRef();

            const REFERENCE_TIME duration = FrameDurationLocked();
            start = stream_start_ + static_cast<REFERENCE_TIME>(frame_index_) * duration;
            stop = start + duration;
            ++frame_index_;

            last_frame_ = frame;
            has_last_frame_ = true;
            last_frame_sequence_ = sequence;
        }

        IMediaSample* sample = nullptr;
        const HRESULT buffer_hr = allocator->GetBuffer(&sample, &start, &stop, 0);
        allocator->Release();

        if (FAILED(buffer_hr) || sample == nullptr) {
            sink->Release();
            continue;
        }

        LONG bytes_written = 0;
        if (CopyFrameToSample(frame, sample, &bytes_written)) {
            sample->SetActualDataLength(bytes_written);
            sample->SetTime(&start, &stop);
            sample->SetSyncPoint(TRUE);
            sample->SetPreroll(FALSE);
            sample->SetDiscontinuity(FALSE);
            sink->Receive(sample);
        }

        sample->Release();
        sink->Release();
    }
}

bool PhoneCamOutputPin::CopyFrameToSample(const PhoneCamFrame& frame, IMediaSample* sample, LONG* bytes_written) {
    if (sample == nullptr || bytes_written == nullptr) {
        return false;
    }

    StreamFormat format{};
    {
        std::lock_guard<std::mutex> guard(lock_);
        format = current_format_;
    }

    if (frame.width != static_cast<std::uint32_t>(format.width) || frame.height != static_cast<std::uint32_t>(format.height)) {
        return false;
    }

    BYTE* destination = nullptr;
    if (FAILED(sample->GetPointer(&destination)) || destination == nullptr) {
        return false;
    }

    const LONG destination_size = sample->GetSize();
    const LONG expected = FrameBufferSizeForFormat(format);
    if (destination_size < expected || expected <= 0) {
        return false;
    }

    if (format.subtype == MEDIASUBTYPE_NV12) {
        if (frame.payload.size() < static_cast<std::size_t>(expected)) {
            return false;
        }

        CopyMemory(destination, frame.payload.data(), static_cast<SIZE_T>(expected));
        *bytes_written = expected;
        return true;
    }

    if (!ConvertNv12ToYuy2(frame, destination, destination_size)) {
        return false;
    }

    *bytes_written = expected;
    return true;
}

bool PhoneCamOutputPin::ConvertNv12ToYuy2(const PhoneCamFrame& frame, BYTE* destination, LONG destination_size) const {
    if (destination == nullptr || destination_size <= 0 || frame.width == 0 || frame.height == 0) {
        return false;
    }

    const std::uint32_t width = frame.width;
    const std::uint32_t height = frame.height;
    if ((width % 2) != 0 || (height % 2) != 0) {
        return false;
    }

    const std::size_t y_plane_size = static_cast<std::size_t>(width) * static_cast<std::size_t>(height);
    const std::size_t uv_plane_size = y_plane_size / 2;
    const std::size_t expected_nv12 = y_plane_size + uv_plane_size;
    const std::size_t expected_yuy2 = y_plane_size * 2;

    if (frame.payload.size() < expected_nv12 || destination_size < static_cast<LONG>(expected_yuy2)) {
        return false;
    }

    const BYTE* y_plane = frame.payload.data();
    const BYTE* uv_plane = y_plane + y_plane_size;

    for (std::uint32_t row = 0; row < height; ++row) {
        const std::size_t y_row_offset = static_cast<std::size_t>(row) * width;
        const std::size_t uv_row_offset = static_cast<std::size_t>(row / 2) * width;
        BYTE* dst_row = destination + static_cast<std::size_t>(row) * width * 2;

        for (std::uint32_t col = 0; col < width; col += 2) {
            const BYTE y0 = y_plane[y_row_offset + col];
            const BYTE y1 = y_plane[y_row_offset + col + 1];
            const BYTE u = uv_plane[uv_row_offset + col];
            const BYTE v = uv_plane[uv_row_offset + col + 1];

            const std::size_t out = static_cast<std::size_t>(col) * 2;
            dst_row[out + 0] = y0;
            dst_row[out + 1] = u;
            dst_row[out + 2] = y1;
            dst_row[out + 3] = v;
        }
    }

    return true;
}

void PhoneCamOutputPin::FreeMediaType(AM_MEDIA_TYPE* media_type) {
    ReleaseMediaType(media_type);
}

HRESULT PhoneCamOutputPin::CopyMediaType(AM_MEDIA_TYPE* destination, const AM_MEDIA_TYPE* source) {
    return CloneMediaType(destination, source);
}

#endif
