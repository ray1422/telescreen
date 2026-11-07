#pragma once

#include <cstdint>

#include "rust/cxx.h"

bool ts_platform_is_wayland();
std::uintptr_t ts_wl_display();
std::uintptr_t ts_window_surface();
bool ts_window_active();
void ts_set_window_visible(bool visible);
void ts_request_activate();
void ts_start_timer(std::int32_t ms);
void ts_quit();
