// SPDX-License-Identifier: GPL-3.0-or-later
//! Minimal ABI subset, checked against mpv 0.41 public client/render headers.
use std::ffi::{c_char, c_int, c_void};
pub type Handle = c_void;
pub type RenderContext = c_void;
#[repr(C)]
pub union NodeData {
    pub string: *mut c_char,
    pub list: *mut NodeList,
    pub int64: i64,
    pub double: f64,
}
#[repr(C)]
pub struct Node {
    pub data: NodeData,
    pub format: c_int,
}
#[repr(C)]
pub struct NodeList {
    pub num: c_int,
    pub values: *mut Node,
    pub keys: *mut *mut c_char,
}
#[repr(C)]
pub struct Event {
    pub id: c_int,
    pub error: c_int,
    pub userdata: u64,
    pub data: *mut c_void,
}
#[repr(C)]
pub struct Property {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}
#[repr(C)]
pub struct StartFile {
    pub playlist_entry_id: i64,
}
#[repr(C)]
pub struct CommandReply {
    pub result: Node,
}
#[repr(C)]
pub struct EndFile {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}
#[repr(C)]
pub struct RenderParam {
    pub kind: c_int,
    pub data: *mut c_void,
}
impl RenderParam {
    pub fn new<T>(kind: i32, data: &mut T) -> Self {
        Self {
            kind,
            data: std::ptr::from_mut(data).cast(),
        }
    }
    pub fn end() -> Self {
        Self {
            kind: 0,
            data: std::ptr::null_mut(),
        }
    }
}
#[repr(C)]
pub struct GlInit {
    pub get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    pub context: *mut c_void,
}
#[repr(C)]
pub struct Fbo {
    pub fbo: c_int,
    pub width: c_int,
    pub height: c_int,
    pub internal_format: c_int,
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct FrameInfo {
    pub flags: u64,
    pub target_time: i64,
}
unsafe extern "C" {
    pub fn mpv_get_time_ns(ctx: *mut Handle) -> i64;
    pub fn mpv_create() -> *mut Handle;
    pub fn mpv_initialize(ctx: *mut Handle) -> c_int;
    pub fn mpv_request_event(ctx: *mut Handle, event: c_int, enable: c_int) -> c_int;
    pub fn mpv_terminate_destroy(ctx: *mut Handle);
    pub fn mpv_set_option_string(
        ctx: *mut Handle,
        name: *const c_char,
        value: *const c_char,
    ) -> c_int;
    pub fn mpv_command_async(ctx: *mut Handle, reply: u64, args: *const *const c_char) -> c_int;
    pub fn mpv_command_node_async(ctx: *mut Handle, reply: u64, args: *mut Node) -> c_int;
    pub fn mpv_observe_property(
        ctx: *mut Handle,
        reply: u64,
        name: *const c_char,
        format: c_int,
    ) -> c_int;
    pub fn mpv_get_property_async(
        ctx: *mut Handle,
        reply: u64,
        name: *const c_char,
        format: c_int,
    ) -> c_int;
    pub fn mpv_set_wakeup_callback(
        ctx: *mut Handle,
        callback: Option<unsafe extern "C" fn(*mut c_void)>,
        data: *mut c_void,
    );
    pub fn mpv_wait_event(ctx: *mut Handle, timeout: f64) -> *mut Event;
    pub fn mpv_render_context_create(
        ctx: *mut *mut RenderContext,
        mpv: *mut Handle,
        params: *mut RenderParam,
    ) -> c_int;
    pub fn mpv_render_context_set_update_callback(
        ctx: *mut RenderContext,
        callback: Option<unsafe extern "C" fn(*mut c_void)>,
        data: *mut c_void,
    );
    pub fn mpv_render_context_update(ctx: *mut RenderContext) -> u64;
    pub fn mpv_render_context_get_info(ctx: *mut RenderContext, param: RenderParam) -> c_int;
    pub fn mpv_render_context_render(ctx: *mut RenderContext, params: *mut RenderParam) -> c_int;
    pub fn mpv_render_context_free(ctx: *mut RenderContext);
}
