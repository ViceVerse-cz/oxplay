/* SPDX-License-Identifier: GPL-3.0-or-later
 * Compile with the private prefix's include directory. Define CHECK_VULKAN
 * for Vulkan; otherwise verify the D3D11 ABI (no Windows SDK required).
 * The application distribution targets 64-bit desktop systems. */
#include <stddef.h>
#include <stdint.h>
#ifdef CHECK_VULKAN
#include <vulkan/vulkan.h>
#include <mpv/render_vk.h>
_Static_assert(MPV_RENDER_PARAM_VULKAN_INIT_PARAMS == 21, "Vulkan init ID");
_Static_assert(MPV_RENDER_PARAM_VULKAN_FBO == 22, "Vulkan FBO ID");
_Static_assert(sizeof(mpv_vulkan_init_params) == 128, "Vulkan init size");
_Static_assert(offsetof(mpv_vulkan_init_params, features) == 88, "features offset");
_Static_assert(offsetof(mpv_vulkan_init_params, drm_render_node) == 120, "DRM offset");
_Static_assert(sizeof(mpv_vulkan_fbo) == 72, "Vulkan FBO size");
_Static_assert(offsetof(mpv_vulkan_fbo, target_layout) == 28, "layout offset");
_Static_assert(offsetof(mpv_vulkan_fbo, signal_value) == 56, "signal offset");
_Static_assert(offsetof(mpv_vulkan_fbo, result) == 64, "status offset");
#else
#include <mpv/render_d3d11.h>
_Static_assert(MPV_RENDER_PARAM_D3D11_INIT_PARAMS == 25, "D3D11 init ID");
_Static_assert(MPV_RENDER_PARAM_D3D11_FBO == 26, "D3D11 FBO ID");
_Static_assert(sizeof(mpv_d3d11_init_params) == 8, "D3D11 init size");
_Static_assert(sizeof(mpv_d3d11_fbo) == 56, "D3D11 FBO size");
_Static_assert(offsetof(mpv_d3d11_fbo, wait_fence) == 16, "wait offset");
_Static_assert(offsetof(mpv_d3d11_fbo, signal_value) == 40, "signal offset");
_Static_assert(offsetof(mpv_d3d11_fbo, result) == 48, "status offset");
#endif
_Static_assert(sizeof(void *) == 8, "64-bit desktop ABI");
_Static_assert(OXPLAY_NATIVE_RENDER_ABI == 1, "Native ABI version");
_Static_assert(MPV_RENDER_PARAM_METAL_BUSY == 27, "Capacity ID");
