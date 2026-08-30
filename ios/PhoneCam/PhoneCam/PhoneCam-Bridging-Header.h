#ifndef PhoneCam_Bridging_Header_h
#define PhoneCam_Bridging_Header_h

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// Raw frame path ABI (shared with Android):
void phonecam_send_video_frame(const uint8_t *data, size_t len, uint64_t pts, bool is_keyframe);
bool phonecam_transport_init(const char *host, uint16_t port);
void phonecam_transport_shutdown(void);
bool phonecam_transport_is_connected(void);
int8_t phonecam_poll_switch_camera_command(void);
uint64_t phonecam_poll_control_command(void);
void phonecam_set_video_dimensions(uint16_t width, uint16_t height);
char *phonecam_parse_qr_code_uri(const char *uri);
char *phonecam_discover_desktops(uint32_t timeout_ms);

// Lightweight FFI test helpers used during bootstrap.
char *phonecam_ffi_test_message(void);
void phonecam_string_free(char *ptr);

// UniFFI-generated C FFI surface.
#if __has_include("Generated/phonecam_mobile_coreFFI.h")
#import "Generated/phonecam_mobile_coreFFI.h"
#elif __has_include("phonecam_mobile_coreFFI.h")
#import "phonecam_mobile_coreFFI.h"
#endif

#endif
