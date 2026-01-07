extern crate libc;
extern crate x11;
use super::common::{FALSE, KEYBOARD, convert};
use super::keyboard::Keyboard;
use crate::rdev::{Event, ListenError};
use libc::c_void;
use std::convert::TryInto;
use std::os::raw::{c_char, c_int, c_uchar, c_uint, c_ulong};
use std::ptr::null;
use x11::xlib;
use x11::xrecord;
use x11::xrecord::XRecordContext;
use x11::xrecord::XRecordDisableContext;

//I would love to get rid of these globals, but some of them are vexxing me - matt.
//I think these globals should be protected by a mutex - there is threading happening
//TODO find a way to remove the globals
static mut RECORD_ALL_CLIENTS: c_ulong = xrecord::XRecordAllClients;
static mut GLOBAL_CALLBACK: Option<Box<dyn FnMut(Event)>> = None;
static mut GLOBAL_RECORD_CONTEXT: Option<XRecordContext> = None;
// This feels dirty, but i didn't want to re-factor to get rid of a few globals
static mut GLOBAL_DISPLAY: Option<*mut xlib::Display> = None;

pub fn listen<T>(callback: T) -> Result<(), ListenError>
where
    T: FnMut(Event) + 'static,
{
    let keyboard = Keyboard::new().ok_or(ListenError::KeyboardError)?;

    unsafe {
        KEYBOARD = Some(keyboard);
        GLOBAL_CALLBACK = Some(Box::new(callback));
        // Open displays
        let dpy_control = xlib::XOpenDisplay(null());
        if dpy_control.is_null() {
            return Err(ListenError::MissingDisplayError);
        }
        //let extension_name = c"RECORD";
        //let extension = xlib::XInitExtension(dpy_control, extension_name.as_ptr());
        //if extension.is_null() {
        //    return Err(ListenError::XRecordExtensionError);
        // }

        // Prepare record range
        let mut record_range: xrecord::XRecordRange = *xrecord::XRecordAllocRange();
        record_range.device_events.first = xlib::KeyPress as c_uchar;
        record_range.device_events.last = xlib::MotionNotify as c_uchar;

        // Create context
        let ptr = &raw mut RECORD_ALL_CLIENTS;
        let context = xrecord::XRecordCreateContext(
            dpy_control,
            0,
            &mut *ptr,
            1,
            &mut &mut record_range as *mut &mut xrecord::XRecordRange
                as *mut *mut xrecord::XRecordRange,
            1,
        );

        if context == 0 {
            return Err(ListenError::RecordContextError);
        }
        GLOBAL_RECORD_CONTEXT = Some(context);
        GLOBAL_DISPLAY = Some(dpy_control);

        xlib::XSync(dpy_control, FALSE);
        // Run
        let result = xrecord::XRecordEnableContext(
            GLOBAL_DISPLAY.unwrap(),
            GLOBAL_RECORD_CONTEXT.unwrap(),
            Some(record_callback),
            &mut 0,
        );
        // Free the record_range
        xlib::XFree(&mut record_range as *mut _ as *mut c_void);
        if result == 0 {
            return Err(ListenError::RecordContextEnablingError);
        }
    }
    Ok(())
}

/// Stops the listen() blocking action
/// It is taking a few seconds to return control on an i7
pub fn stop_listening() {
    unsafe {
        if let Some(c) = GLOBAL_RECORD_CONTEXT {
            if XRecordDisableContext(GLOBAL_DISPLAY.unwrap(), c) != 0 {
                GLOBAL_RECORD_CONTEXT = None;
            }
        }
    }
}

// No idea how to do that properly relevant doc lives here:
// https://www.x.org/releases/X11R7.7/doc/libXtst/recordlib.html#Datum_Flags
// https://docs.rs/xproto/1.1.5/xproto/struct._xEvent__bindgen_ty_1.html
// 0.4.2: xproto was removed for some reason and contained the real structs
// but we can't use it anymore.
#[repr(C)]
struct XRecordDatum {
    type_: u8,
    code: u8,
    _rest: u64,
    _1: bool,
    _2: bool,
    _3: bool,
    root_x: i16,
    root_y: i16,
    event_x: i16,
    event_y: i16,
    state: u16,
}

unsafe extern "C" fn record_callback(
    _null: *mut c_char,
    raw_data: *mut xrecord::XRecordInterceptData,
) {
    unsafe {
        let data = raw_data.as_ref().unwrap();
        if data.category != xrecord::XRecordFromServer {
            return;
        }

        debug_assert!(data.data_len * 4 >= std::mem::size_of::<XRecordDatum>().try_into().unwrap());
        // Cast binary data
        #[allow(clippy::cast_ptr_alignment)]
        let xdatum = (data.data as *const XRecordDatum).as_ref().unwrap();

        let code: c_uint = xdatum.code.into();
        let type_: c_int = xdatum.type_.into();

        let x = xdatum.root_x as f64;
        let y = xdatum.root_y as f64;

        let ptr = &raw mut KEYBOARD;
        if let Some(event) = convert(&mut *ptr, code, type_, x, y) {
            let ptr = &raw mut GLOBAL_CALLBACK;
            if let Some(callback) = &mut *ptr {
                callback(event);
            }
        }
        xrecord::XRecordFreeData(raw_data);
    }
}
