#ifndef PhoneCam_Bridging_Header_h
#define PhoneCam_Bridging_Header_h

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// Typed profile/frame ABI shared with Android.
bool phonecam_send_video_frame(const uint8_t *data, size_t len, uint64_t pts,
                               uint8_t codec, uint16_t width, uint16_t height,
                               bool is_keyframe);
bool phonecam_transport_init(const char *host, uint16_t port,
                             const char *video_config_json);
void phonecam_transport_shutdown(void);
bool phonecam_transport_is_connected(void);
char *phonecam_poll_control_command_json(void);
bool phonecam_peer_supports_profile(uint8_t codec, uint16_t width,
                                    uint16_t height, uint8_t fps);
bool phonecam_update_video_capabilities(const char *profiles_json);
bool phonecam_report_stream_configuration(uint32_t request_id,
                                          uint8_t result_code, uint8_t codec,
                                          uint16_t width, uint16_t height,
                                          uint8_t fps);
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
