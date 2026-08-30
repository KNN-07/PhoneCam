#if defined(_WIN32)

#include <windows.h>
#include <initguid.h>
#include <cguid.h>
#include <dshow.h>

#include <atomic>
#include <functional>
#include <new>

#include "guids.h"
#include "module.h"

extern "C" HRESULT WINAPI AMovieDllRegisterServer2(BOOL register_filter);
extern HRESULT PhoneCamCreateFilterInstance(REFIID riid, void** ppv);

namespace {

std::atomic<long> g_module_ref_count{0};
HMODULE g_module_handle = nullptr;

class PhoneCamClassFactory final : public IClassFactory {
  public:
    PhoneCamClassFactory() : ref_count_(1) {}

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
        if (ppv == nullptr) {
            return E_POINTER;
        }

        if (riid == IID_IUnknown || riid == IID_IClassFactory) {
            *ppv = static_cast<IClassFactory*>(this);
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

    STDMETHODIMP CreateInstance(IUnknown* outer, REFIID riid, void** ppv) override {
        if (ppv == nullptr) {
            return E_POINTER;
        }

        *ppv = nullptr;

        if (outer != nullptr) {
            return CLASS_E_NOAGGREGATION;
        }

        return PhoneCamCreateFilterInstance(riid, ppv);
    }

    STDMETHODIMP LockServer(BOOL lock) override {
        if (lock) {
            PhoneCamModuleAddRef();
        } else {
            PhoneCamModuleRelease();
        }

        return S_OK;
    }

  private:
    volatile long ref_count_;
};

HRESULT RegisterCaptureFilterCategory() {
    IFilterMapper2* mapper = nullptr;
    HRESULT hr = CoCreateInstance(
        CLSID_FilterMapper2,
        nullptr,
        CLSCTX_INPROC_SERVER,
        IID_IFilterMapper2,
        reinterpret_cast<void**>(&mapper));
    if (FAILED(hr)) {
        return hr;
    }

    REGPINTYPES pin_types[1] = {};
    pin_types[0].clsMajorType = &MEDIATYPE_Video;
    pin_types[0].clsMinorType = &MEDIASUBTYPE_NULL;

    REGFILTERPINS2 pin = {};
    pin.dwFlags = REG_PINFLAG_B_OUTPUT;
    pin.cInstances = 1;
    pin.nMediaTypes = 1;
    pin.lpMediaType = pin_types;
    pin.nMediums = 0;
    pin.lpMedium = nullptr;
    pin.clsPinCategory = const_cast<CLSID*>(&PIN_CATEGORY_CAPTURE);

    REGFILTER2 filter = {};
    filter.dwVersion = 2;
    filter.dwMerit = MERIT_DO_NOT_USE;
    filter.cPins2 = 1;
    filter.rgPins2 = &pin;

    hr = mapper->RegisterFilter(
        CLSID_PhoneCamFilter,
        kPhoneCamFilterName,
        nullptr,
        &CLSID_VideoInputDeviceCategory,
        nullptr,
        &filter);

    mapper->Release();
    return hr;
}

HRESULT UnregisterCaptureFilterCategory() {
    IFilterMapper2* mapper = nullptr;
    HRESULT hr = CoCreateInstance(
        CLSID_FilterMapper2,
        nullptr,
        CLSCTX_INPROC_SERVER,
        IID_IFilterMapper2,
        reinterpret_cast<void**>(&mapper));
    if (FAILED(hr)) {
        return hr;
    }

    hr = mapper->UnregisterFilter(&CLSID_VideoInputDeviceCategory, nullptr, CLSID_PhoneCamFilter);
    mapper->Release();
    return hr;
}

HRESULT WithComApartment(const std::function<HRESULT()>& work) {
    HRESULT init_hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (init_hr == RPC_E_CHANGED_MODE) {
        init_hr = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    }

    const bool should_uninitialize = SUCCEEDED(init_hr);
    if (FAILED(init_hr) && init_hr != RPC_E_CHANGED_MODE) {
        return init_hr;
    }

    const HRESULT work_hr = work();

    if (should_uninitialize) {
        CoUninitialize();
    }

    return work_hr;
}

}

void PhoneCamModuleAddRef() {
    g_module_ref_count.fetch_add(1, std::memory_order_relaxed);
}

void PhoneCamModuleRelease() {
    g_module_ref_count.fetch_sub(1, std::memory_order_relaxed);
}

long PhoneCamModuleRefCount() {
    return g_module_ref_count.load(std::memory_order_relaxed);
}

HMODULE PhoneCamModuleHandle() {
    return g_module_handle;
}

void PhoneCamSetModuleHandle(HMODULE module_handle) {
    g_module_handle = module_handle;
}

extern "C" BOOL APIENTRY DllMain(HMODULE module, DWORD reason, LPVOID reserved) {
    if (reason == DLL_PROCESS_ATTACH) {
        DisableThreadLibraryCalls(module);
        PhoneCamSetModuleHandle(module);
    }

    if (reason == DLL_PROCESS_DETACH) {
        if (reserved == nullptr) {
            PhoneCamSetModuleHandle(nullptr);
        }
    }

    return TRUE;
}

extern "C" STDAPI DllCanUnloadNow(void) {
    return PhoneCamModuleRefCount() == 0 ? S_OK : S_FALSE;
}

extern "C" STDAPI DllGetClassObject(REFCLSID clsid, REFIID riid, void** ppv) {
    if (ppv == nullptr) {
        return E_POINTER;
    }

    *ppv = nullptr;

    if (clsid != CLSID_PhoneCamFilter) {
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    PhoneCamClassFactory* factory = new (std::nothrow) PhoneCamClassFactory();
    if (factory == nullptr) {
        return E_OUTOFMEMORY;
    }

    const HRESULT hr = factory->QueryInterface(riid, ppv);
    factory->Release();
    return hr;
}

extern "C" STDAPI DllRegisterServer(void) {
    HRESULT hr = AMovieDllRegisterServer2(TRUE);
    if (FAILED(hr)) {
        return hr;
    }

    hr = WithComApartment([]() { return RegisterCaptureFilterCategory(); });
    if (FAILED(hr)) {
        AMovieDllRegisterServer2(FALSE);
    }

    return hr;
}

extern "C" STDAPI DllUnregisterServer(void) {
    const HRESULT category_hr = WithComApartment([]() { return UnregisterCaptureFilterCategory(); });
    const HRESULT unregister_hr = AMovieDllRegisterServer2(FALSE);

    if (FAILED(unregister_hr)) {
        return unregister_hr;
    }

    if (FAILED(category_hr) && category_hr != HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)) {
        return category_hr;
    }

    return S_OK;
}

#else

extern "C" int DllMain(void*, unsigned long, void*) {
    return 1;
}

extern "C" long DllCanUnloadNow() {
    return 1;
}

extern "C" long DllGetClassObject(const void*, const void*, void**) {
    return 0x80040111L;
}

extern "C" long DllRegisterServer() {
    return 0;
}

extern "C" long DllUnregisterServer() {
    return 0;
}

#endif
