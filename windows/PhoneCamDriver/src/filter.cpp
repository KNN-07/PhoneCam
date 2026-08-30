#include "filter.h"

#if defined(_WIN32)

#include "guids.h"
#include "module.h"
#include "output_pin.h"

#include <new>

namespace {

class SinglePinEnum final : public IEnumPins {
  public:
    explicit SinglePinEnum(IPin* pin) : ref_count_(1), pin_(pin), index_(0) {
        if (pin_ != nullptr) {
            pin_->AddRef();
        }
    }

    ~SinglePinEnum() {
        if (pin_ != nullptr) {
            pin_->Release();
            pin_ = nullptr;
        }
    }

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
        if (ppv == nullptr) {
            return E_POINTER;
        }

        if (riid == IID_IUnknown || riid == IID_IEnumPins) {
            *ppv = static_cast<IEnumPins*>(this);
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

    STDMETHODIMP Next(ULONG cPins, IPin** ppPins, ULONG* pcFetched) override {
        if (ppPins == nullptr) {
            return E_POINTER;
        }

        if (cPins > 1 && pcFetched == nullptr) {
            return E_POINTER;
        }

        ULONG fetched = 0;
        while (fetched < cPins && index_ < 1 && pin_ != nullptr) {
            ppPins[fetched] = pin_;
            ppPins[fetched]->AddRef();
            ++fetched;
            ++index_;
        }

        if (pcFetched != nullptr) {
            *pcFetched = fetched;
        }

        return fetched == cPins ? S_OK : S_FALSE;
    }

    STDMETHODIMP Skip(ULONG cPins) override {
        const ULONG remaining = index_ < 1 ? (1 - index_) : 0;
        const ULONG skipped = cPins < remaining ? cPins : remaining;
        index_ += skipped;
        return skipped == cPins ? S_OK : S_FALSE;
    }

    STDMETHODIMP Reset() override {
        index_ = 0;
        return S_OK;
    }

    STDMETHODIMP Clone(IEnumPins** ppEnum) override {
        if (ppEnum == nullptr) {
            return E_POINTER;
        }

        *ppEnum = nullptr;

        SinglePinEnum* clone = new (std::nothrow) SinglePinEnum(pin_);
        if (clone == nullptr) {
            return E_OUTOFMEMORY;
        }

        clone->index_ = index_;
        *ppEnum = clone;
        return S_OK;
    }

  private:
    volatile long ref_count_;
    IPin* pin_;
    ULONG index_;
};

bool IsOutputPinId(LPCWSTR id) {
    if (id == nullptr) {
        return false;
    }

    return lstrcmpiW(id, L"Output") == 0 || lstrcmpiW(id, kPhoneCamOutputPinName) == 0;
}

} 

PhoneCamFilter::PhoneCamFilter()
    : ref_count_(1), reference_clock_(nullptr), filter_graph_(nullptr), output_pin_(nullptr) {
    filter_name_[0] = L'\0';
    lstrcpynW(filter_name_, kPhoneCamFilterName, static_cast<int>(sizeof(filter_name_) / sizeof(filter_name_[0])));
    output_pin_ = new (std::nothrow) PhoneCamOutputPin(this);
    PhoneCamModuleAddRef();
}

PhoneCamFilter::~PhoneCamFilter() {
    if (output_pin_ != nullptr) {
        delete output_pin_;
        output_pin_ = nullptr;
    }

    if (reference_clock_ != nullptr) {
        reference_clock_->Release();
        reference_clock_ = nullptr;
    }

    if (filter_graph_ != nullptr) {
        filter_graph_->Release();
        filter_graph_ = nullptr;
    }

    PhoneCamModuleRelease();
}

HRESULT PhoneCamFilter::CreateInstance(REFIID riid, void** ppv) {
    if (ppv == nullptr) {
        return E_POINTER;
    }

    *ppv = nullptr;

    PhoneCamFilter* filter = new (std::nothrow) PhoneCamFilter();
    if (filter == nullptr) {
        return E_OUTOFMEMORY;
    }

    if (filter->output_pin_ == nullptr) {
        filter->Release();
        return E_OUTOFMEMORY;
    }

    const HRESULT hr = filter->QueryInterface(riid, ppv);
    filter->Release();
    return hr;
}

STDMETHODIMP PhoneCamFilter::QueryInterface(REFIID riid, void** ppv) {
    if (ppv == nullptr) {
        return E_POINTER;
    }

    if (riid == IID_IUnknown || riid == IID_IPersist || riid == IID_IMediaFilter || riid == IID_IBaseFilter) {
        *ppv = static_cast<IBaseFilter*>(this);
    } else if (riid == IID_ISpecifyPropertyPages) {
        *ppv = static_cast<ISpecifyPropertyPages*>(this);
    } else {
        *ppv = nullptr;
        return E_NOINTERFACE;
    }

    AddRef();
    return S_OK;
}

STDMETHODIMP_(ULONG) PhoneCamFilter::AddRef() {
    return ref_count_.fetch_add(1, std::memory_order_relaxed) + 1;
}

STDMETHODIMP_(ULONG) PhoneCamFilter::Release() {
    const ULONG count = ref_count_.fetch_sub(1, std::memory_order_acq_rel) - 1;
    if (count == 0) {
        delete this;
    }

    return count;
}

STDMETHODIMP PhoneCamFilter::GetClassID(CLSID* class_id) {
    if (class_id == nullptr) {
        return E_POINTER;
    }

    *class_id = CLSID_PhoneCamFilter;
    return S_OK;
}

STDMETHODIMP PhoneCamFilter::Stop() {
    PhoneCamOutputPin* pin = nullptr;
    {
        std::lock_guard<std::mutex> guard(lock_);
        state_ = State_Stopped;
        pin = output_pin_;
    }

    return pin != nullptr ? pin->StopStreaming() : E_FAIL;
}

STDMETHODIMP PhoneCamFilter::Pause() {
    PhoneCamOutputPin* pin = nullptr;
    {
        std::lock_guard<std::mutex> guard(lock_);
        state_ = State_Paused;
        pin = output_pin_;
    }

    return pin != nullptr ? pin->PauseStreaming() : E_FAIL;
}

STDMETHODIMP PhoneCamFilter::Run(REFERENCE_TIME start_time) {
    PhoneCamOutputPin* pin = nullptr;
    {
        std::lock_guard<std::mutex> guard(lock_);
        state_ = State_Running;
        pin = output_pin_;
    }

    return pin != nullptr ? pin->StartStreaming(start_time) : E_FAIL;
}

STDMETHODIMP PhoneCamFilter::GetState(DWORD, FILTER_STATE* state) {
    if (state == nullptr) {
        return E_POINTER;
    }

    std::lock_guard<std::mutex> guard(lock_);
    *state = state_;
    return S_OK;
}

STDMETHODIMP PhoneCamFilter::SetSyncSource(IReferenceClock* clock) {
    std::lock_guard<std::mutex> guard(lock_);

    if (clock != nullptr) {
        clock->AddRef();
    }

    if (reference_clock_ != nullptr) {
        reference_clock_->Release();
    }

    reference_clock_ = clock;
    return S_OK;
}

STDMETHODIMP PhoneCamFilter::GetSyncSource(IReferenceClock** clock) {
    if (clock == nullptr) {
        return E_POINTER;
    }

    std::lock_guard<std::mutex> guard(lock_);
    *clock = reference_clock_;
    if (*clock != nullptr) {
        (*clock)->AddRef();
    }

    return S_OK;
}

STDMETHODIMP PhoneCamFilter::EnumPins(IEnumPins** enum_pins) {
    if (enum_pins == nullptr) {
        return E_POINTER;
    }

    *enum_pins = nullptr;

    IPin* pin = GetPin(0);
    if (pin == nullptr) {
        return E_FAIL;
    }

    SinglePinEnum* enumerator = new (std::nothrow) SinglePinEnum(pin);
    if (enumerator == nullptr) {
        return E_OUTOFMEMORY;
    }

    *enum_pins = enumerator;
    return S_OK;
}

STDMETHODIMP PhoneCamFilter::FindPin(LPCWSTR id, IPin** pin) {
    if (pin == nullptr) {
        return E_POINTER;
    }

    *pin = nullptr;

    if (!IsOutputPinId(id)) {
        return VFW_E_NOT_FOUND;
    }

    IPin* output = GetPin(0);
    if (output == nullptr) {
        return E_FAIL;
    }

    output->AddRef();
    *pin = output;
    return S_OK;
}

STDMETHODIMP PhoneCamFilter::QueryFilterInfo(FILTER_INFO* filter_info) {
    if (filter_info == nullptr) {
        return E_POINTER;
    }

    std::lock_guard<std::mutex> guard(lock_);
    lstrcpynW(
        filter_info->achName,
        filter_name_,
        static_cast<int>(sizeof(filter_info->achName) / sizeof(filter_info->achName[0])));

    filter_info->pGraph = filter_graph_;
    if (filter_info->pGraph != nullptr) {
        filter_info->pGraph->AddRef();
    }

    return S_OK;
}

STDMETHODIMP PhoneCamFilter::JoinFilterGraph(IFilterGraph* graph, LPCWSTR name) {
    std::lock_guard<std::mutex> guard(lock_);

    if (filter_graph_ != nullptr) {
        filter_graph_->Release();
        filter_graph_ = nullptr;
    }

    if (graph != nullptr) {
        graph->AddRef();
        filter_graph_ = graph;
    }

    if (name != nullptr && name[0] != L'\0') {
        lstrcpynW(filter_name_, name, static_cast<int>(sizeof(filter_name_) / sizeof(filter_name_[0])));
    } else {
        lstrcpynW(
            filter_name_,
            kPhoneCamFilterName,
            static_cast<int>(sizeof(filter_name_) / sizeof(filter_name_[0])));
    }

    return S_OK;
}

STDMETHODIMP PhoneCamFilter::QueryVendorInfo(LPWSTR* vendor_info) {
    if (vendor_info == nullptr) {
        return E_POINTER;
    }

    *vendor_info = nullptr;

    const UINT char_count = lstrlenW(kPhoneCamVendorName) + 1;
    const SIZE_T bytes = static_cast<SIZE_T>(char_count) * sizeof(WCHAR);
    LPWSTR buffer = static_cast<LPWSTR>(CoTaskMemAlloc(bytes));
    if (buffer == nullptr) {
        return E_OUTOFMEMORY;
    }

    CopyMemory(buffer, kPhoneCamVendorName, bytes);
    *vendor_info = buffer;
    return S_OK;
}

STDMETHODIMP PhoneCamFilter::GetPages(CAUUID* pages) {
    if (pages == nullptr) {
        return E_POINTER;
    }

    pages->cElems = 0;
    pages->pElems = nullptr;
    return E_NOTIMPL;
}

int PhoneCamFilter::GetPinCount() const {
    std::lock_guard<std::mutex> guard(lock_);
    return output_pin_ != nullptr ? 1 : 0;
}

IPin* PhoneCamFilter::GetPin(int index) const {
    std::lock_guard<std::mutex> guard(lock_);
    if (index != 0 || output_pin_ == nullptr) {
        return nullptr;
    }

    return static_cast<IPin*>(output_pin_);
}

HRESULT PhoneCamCreateFilterInstance(REFIID riid, void** ppv) {
    return PhoneCamFilter::CreateInstance(riid, ppv);
}

#endif
