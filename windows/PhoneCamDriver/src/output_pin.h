#pragma once

#if defined(_WIN32)

#include <dshow.h>
#include <ks.h>
#include <ksproxy.h>

#include <atomic>
#include <cstdint>
#include <mutex>
#include <thread>

#include "frame_receiver.h"

class PhoneCamFilter;

class PhoneCamOutputPin final : public IPin,
                                public IAMStreamConfig,
                                public IKsPropertySet,
                                public IMemInputPin {
  public:
    explicit PhoneCamOutputPin(PhoneCamFilter* filter);
    ~PhoneCamOutputPin();

    HRESULT StartStreaming(REFERENCE_TIME stream_start);
    HRESULT PauseStreaming();
    HRESULT StopStreaming();
    bool IsConnected() const;

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
    STDMETHODIMP_(ULONG) AddRef() override;
    STDMETHODIMP_(ULONG) Release() override;

    STDMETHODIMP Connect(IPin* pReceivePin, const AM_MEDIA_TYPE* pmt) override;
    STDMETHODIMP ReceiveConnection(IPin* pConnector, const AM_MEDIA_TYPE* pmt) override;
    STDMETHODIMP Disconnect() override;
    STDMETHODIMP ConnectedTo(IPin** ppPin) override;
    STDMETHODIMP ConnectionMediaType(AM_MEDIA_TYPE* pmt) override;
    STDMETHODIMP QueryPinInfo(PIN_INFO* pInfo) override;
    STDMETHODIMP QueryDirection(PIN_DIRECTION* pPinDir) override;
    STDMETHODIMP QueryId(LPWSTR* Id) override;
    STDMETHODIMP QueryAccept(const AM_MEDIA_TYPE* pmt) override;
    STDMETHODIMP EnumMediaTypes(IEnumMediaTypes** ppEnum) override;
    STDMETHODIMP QueryInternalConnections(IPin** apPin, ULONG* nPin) override;
    STDMETHODIMP EndOfStream() override;
    STDMETHODIMP BeginFlush() override;
    STDMETHODIMP EndFlush() override;
    STDMETHODIMP NewSegment(REFERENCE_TIME tStart, REFERENCE_TIME tStop, double dRate) override;

    STDMETHODIMP SetFormat(AM_MEDIA_TYPE* pmt) override;
    STDMETHODIMP GetFormat(AM_MEDIA_TYPE** ppmt) override;
    STDMETHODIMP GetNumberOfCapabilities(int* piCount, int* piSize) override;
    STDMETHODIMP GetStreamCaps(int iIndex, AM_MEDIA_TYPE** ppmt, BYTE* pSCC) override;

    STDMETHODIMP Set(REFGUID guidPropSet, DWORD dwPropID, void* pInstanceData, DWORD cbInstanceData,
                     void* pPropData, DWORD cbPropData) override;
    STDMETHODIMP Get(REFGUID guidPropSet, DWORD dwPropID, void* pInstanceData, DWORD cbInstanceData,
                     void* pPropData, DWORD cbPropData, DWORD* pcbReturned) override;
    STDMETHODIMP QuerySupported(REFGUID guidPropSet, DWORD dwPropID, DWORD* pTypeSupport) override;

    STDMETHODIMP GetAllocator(IMemAllocator** ppAllocator) override;
    STDMETHODIMP NotifyAllocator(IMemAllocator* pAllocator, BOOL bReadOnly) override;
    STDMETHODIMP GetAllocatorRequirements(ALLOCATOR_PROPERTIES* pProps) override;
    STDMETHODIMP Receive(IMediaSample* pSample) override;
    STDMETHODIMP ReceiveMultiple(IMediaSample** pSamples, long nSamples, long* nSamplesProcessed) override;
    STDMETHODIMP ReceiveCanBlock() override;

  private:
    struct StreamFormat {
        LONG width;
        LONG height;
        GUID subtype;
        REFERENCE_TIME avg_time_per_frame;
    };

    HRESULT CompleteConnectionLocked(IPin* peer_pin, const AM_MEDIA_TYPE* media_type);
    HRESULT EnsureAllocatorLocked();
    void ReleaseAllocatorLocked();

    bool IsSupportedSubtype(REFGUID subtype) const;
    bool IsSupportedMediaType(const AM_MEDIA_TYPE* media_type) const;
    HRESULT BuildMediaType(const StreamFormat& format, AM_MEDIA_TYPE* media_type) const;
    HRESULT BuildMediaTypeForIndex(ULONG index, AM_MEDIA_TYPE* media_type) const;
    LONG FrameBufferSizeLocked() const;
    static LONG FrameBufferSizeForFormat(const StreamFormat& format);
    REFERENCE_TIME FrameDurationLocked() const;

    void StreamingLoop();
    bool CopyFrameToSample(const PhoneCamFrame& frame, IMediaSample* sample, LONG* bytes_written);
    bool ConvertNv12ToYuy2(const PhoneCamFrame& frame, BYTE* destination, LONG destination_size) const;

    static void FreeMediaType(AM_MEDIA_TYPE* media_type);
    static HRESULT CopyMediaType(AM_MEDIA_TYPE* destination, const AM_MEDIA_TYPE* source);

    std::atomic<ULONG> ref_count_;
    PhoneCamFilter* filter_;

    mutable std::mutex lock_;
    IPin* connected_pin_;
    IMemInputPin* mem_input_pin_;
    IMemAllocator* allocator_;
    AM_MEDIA_TYPE connected_media_type_;
    bool has_connected_media_type_;
    StreamFormat current_format_;

    FrameReceiver frame_receiver_;
    std::thread streaming_thread_;
    std::atomic<bool> streaming_;
    std::atomic<bool> stop_requested_;
    PhoneCamFrame last_frame_;
    bool has_last_frame_;
    REFERENCE_TIME next_sample_time_;
};

#else

class PhoneCamFilter;

class PhoneCamOutputPin {
  public:
    explicit PhoneCamOutputPin(PhoneCamFilter*) {}
};

#endif
