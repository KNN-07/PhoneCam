#pragma once

#include "output_pin.h"

#if defined(_WIN32)

#include <dshow.h>
#include <ks.h>
#include <ksproxy.h>

#include <atomic>
#include <mutex>

class PhoneCamOutputPin;

class PhoneCamFilter final : public IBaseFilter, public ISpecifyPropertyPages {
  public:
    PhoneCamFilter();
    ~PhoneCamFilter();

    static HRESULT CreateInstance(REFIID riid, void** ppv);

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
    STDMETHODIMP_(ULONG) AddRef() override;
    STDMETHODIMP_(ULONG) Release() override;

    STDMETHODIMP GetClassID(CLSID* class_id) override;

    STDMETHODIMP Stop() override;
    STDMETHODIMP Pause() override;
    STDMETHODIMP Run(REFERENCE_TIME start_time) override;
    STDMETHODIMP GetState(DWORD milliseconds_timeout, FILTER_STATE* state) override;
    STDMETHODIMP SetSyncSource(IReferenceClock* clock) override;
    STDMETHODIMP GetSyncSource(IReferenceClock** clock) override;

    STDMETHODIMP EnumPins(IEnumPins** enum_pins) override;
    STDMETHODIMP FindPin(LPCWSTR id, IPin** pin) override;
    STDMETHODIMP QueryFilterInfo(FILTER_INFO* filter_info) override;
    STDMETHODIMP JoinFilterGraph(IFilterGraph* graph, LPCWSTR name) override;
    STDMETHODIMP QueryVendorInfo(LPWSTR* vendor_info) override;

    STDMETHODIMP GetPages(CAUUID* pages) override;

    int GetPinCount() const;
    IPin* GetPin(int index) const;

  private:
    std::atomic<ULONG> ref_count_;
    mutable std::mutex lock_;
    FILTER_STATE state_ = State_Stopped;
    IReferenceClock* reference_clock_;
    IFilterGraph* filter_graph_;
    WCHAR filter_name_[128];
    PhoneCamOutputPin* output_pin_;
};

HRESULT PhoneCamCreateFilterInstance(REFIID riid, void** ppv);

#else

class PhoneCamFilter {
  public:
    PhoneCamFilter() = default;
    ~PhoneCamFilter() = default;
};

inline long PhoneCamCreateFilterInstance(const void*, void**) {
    return 0;
}

#endif
