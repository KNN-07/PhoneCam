cmake_minimum_required(VERSION 3.20)

project(PhoneCamDriver LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT WIN32)
  message(STATUS "PhoneCamDriver is Windows-only. Skipping native target generation on this platform.")
  add_custom_target(
    PhoneCamDriverStub ALL
    COMMAND ${CMAKE_COMMAND} -E echo "PhoneCamDriver build skipped: requires Windows SDK + DirectShow headers/libs"
  )
  return()
endif()

add_library(
  PhoneCamDriver
  SHARED
  PhoneCamDriver.def
  src/dllmain.cpp
  src/filter.cpp
  src/output_pin.cpp
  src/frame_receiver.cpp
  src/guids.cpp
)

target_include_directories(PhoneCamDriver PRIVATE src)

target_compile_definitions(
  PhoneCamDriver
  PRIVATE
    WIN32_LEAN_AND_MEAN
    NOMINMAX
    UNICODE
    _UNICODE
    _WIN32_WINNT=0x0A00
)

target_link_libraries(
  PhoneCamDriver
  PRIVATE
    strmiids
    ole32
    oleaut32
    advapi32
    user32
    uuid
)

set_target_properties(
  PhoneCamDriver
  PROPERTIES
    OUTPUT_NAME "PhoneCamDriver"
)
