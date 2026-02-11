#pragma once

#include <windows.h>

void PhoneCamModuleAddRef();
void PhoneCamModuleRelease();
long PhoneCamModuleRefCount();
HMODULE PhoneCamModuleHandle();
void PhoneCamSetModuleHandle(HMODULE module_handle);
