#include <initguid.h>

#include "guids.h"

DEFINE_GUID(
    CLSID_PhoneCamFilter,
    0xa3798a9d,
    0x5b2c,
    0x4bc3,
    0xbb,
    0x4a,
    0x2f,
    0x43,
    0xc4,
    0xb2,
    0xa1,
    0xc1);

DEFINE_GUID(
    CLSID_PhoneCamOutputPin,
    0xb8629d90,
    0x1f30,
    0x40fc,
    0xaf,
    0x8b,
    0x0e,
    0x2d,
    0x49,
    0x24,
    0x11,
    0x6a);

DEFINE_GUID(
    CLSID_PhoneCamPropertyPage,
    0x2dd2c52e,
    0x7d71,
    0x4f58,
    0x9d,
    0x2d,
    0xc8,
    0x5d,
    0x7e,
    0x5c,
    0x55,
    0xf4);

const wchar_t kPhoneCamFilterName[] = L"PhoneCam";
const wchar_t kPhoneCamOutputPinName[] = L"Output";
const wchar_t kPhoneCamVendorName[] = L"PhoneCam";
