// Copyright 2020-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::{
  ffi::{c_void, CStr},
  path::PathBuf,
};

use cocoa::{
  base::{id, BOOL, YES},
  foundation::{NSPoint, NSRect},
};
use objc::{
  declare::ClassDecl,
  runtime::{class_getInstanceMethod, method_getImplementation, Object, Sel},
};
use once_cell::sync::Lazy;

use crate::FileDropEvent;

pub(crate) type NSDragOperation = cocoa::foundation::NSUInteger;
#[allow(non_upper_case_globals)]
const NSDragOperationCopy: NSDragOperation = 1;

static OBJC_DRAGGING_ENTERED: Lazy<extern "C" fn(*const Object, Sel, id) -> NSDragOperation> =
  Lazy::new(|| unsafe {
    std::mem::transmute(method_getImplementation(class_getInstanceMethod(
      class!(WKWebView),
      sel!(draggingEntered:),
    )))
  });

static OBJC_DRAGGING_EXITED: Lazy<extern "C" fn(*const Object, Sel, id)> = Lazy::new(|| unsafe {
  std::mem::transmute(method_getImplementation(class_getInstanceMethod(
    class!(WKWebView),
    sel!(draggingExited:),
  )))
});

static OBJC_PERFORM_DRAG_OPERATION: Lazy<extern "C" fn(*const Object, Sel, id) -> BOOL> =
  Lazy::new(|| unsafe {
    std::mem::transmute(method_getImplementation(class_getInstanceMethod(
      class!(WKWebView),
      sel!(performDragOperation:),
    )))
  });

static OBJC_DRAGGING_UPDATED: Lazy<extern "C" fn(*const Object, Sel, id) -> NSDragOperation> =
  Lazy::new(|| unsafe {
    std::mem::transmute(method_getImplementation(class_getInstanceMethod(
      class!(WKWebView),
      sel!(draggingUpdated:),
    )))
  });

// Safety: objc runtime calls are unsafe
pub(crate) unsafe fn set_file_drop_handler(
  webview: *mut Object,
  handler: Box<dyn Fn(FileDropEvent) -> bool>,
) -> *mut Box<dyn Fn(FileDropEvent) -> bool> {
  let listener = Box::into_raw(Box::new(handler));
  (*webview).set_ivar("FileDropHandler", listener as *mut _ as *mut c_void);
  listener
}

#[allow(clippy::mut_from_ref)]
unsafe fn get_handler(this: &Object) -> &mut Box<dyn Fn(FileDropEvent) -> bool> {
  let delegate: *mut c_void = *this.get_ivar("FileDropHandler");
  &mut *(delegate as *mut Box<dyn Fn(FileDropEvent) -> bool>)
}

unsafe fn collect_paths(drag_info: id) -> Vec<PathBuf> {
  use cocoa::{
    appkit::{NSFilenamesPboardType, NSPasteboard},
    foundation::{NSFastEnumeration, NSString},
  };

  let pb: id = msg_send![drag_info, draggingPasteboard];
  let mut file_drop_paths = Vec::new();
  let types: id = msg_send![class!(NSArray), arrayWithObject: NSFilenamesPboardType];
  if !NSPasteboard::availableTypeFromArray(pb, types).is_null() {
    // Suu's patch: a host's own drag (Ableton Live's, of a device or a clip) names the file
    // names type with no list or an empty one. cocoa's fast iterator reads the mutations pointer
    // before it knows there are items, and an empty enumeration leaves it null: it dereferenced
    // null and took the host down. Only an array is read, by index, and only its strings.
    let list: id = NSPasteboard::propertyListForType(pb, NSFilenamesPboardType);
    let is_array: bool = !list.is_null() && msg_send![list, isKindOfClass: class!(NSArray)];
    let count: usize = if is_array { msg_send![list, count] } else { 0 };
    for i in 0..count {
      let path: id = msg_send![list, objectAtIndex: i];
      {
        if path.is_null() {
          continue;
        }
        let is_string: bool = msg_send![path, isKindOfClass: class!(NSString)];
        let utf8 = if is_string { NSString::UTF8String(path) } else { std::ptr::null() };
        if utf8.is_null() {
          continue;
        }
        file_drop_paths.push(PathBuf::from(CStr::from_ptr(utf8).to_string_lossy().into_owned()));
      }
    }
  }
  file_drop_paths
}

extern "C" fn dragging_updated(this: &mut Object, sel: Sel, drag_info: id) -> NSDragOperation {
  let os_operation = OBJC_DRAGGING_UPDATED(this, sel, drag_info);
  if os_operation == 0 {
    // 0 will be returned for a file drop on any arbitrary location on the webview.
    // We'll override that with NSDragOperationCopy.
    NSDragOperationCopy
  } else {
    // A different NSDragOperation is returned when a file is hovered over something like
    // a <input type="file">, so we'll make sure to preserve that behaviour.
    os_operation
  }
}

extern "C" fn dragging_entered(this: &mut Object, sel: Sel, drag_info: id) -> NSDragOperation {
  let listener = unsafe { get_handler(this) };
  let paths = unsafe { collect_paths(drag_info) };

  let dl: NSPoint = unsafe { msg_send![drag_info, draggingLocation] };
  let frame: NSRect = unsafe { msg_send![this, frame] };
  let position = (dl.x as i32, (frame.size.height - dl.y) as i32);

  if !listener(FileDropEvent::Hovered { paths, position }) {
    // Reject the Wry file drop (invoke the OS default behaviour)
    OBJC_DRAGGING_ENTERED(this, sel, drag_info)
  } else {
    NSDragOperationCopy
  }
}

extern "C" fn perform_drag_operation(this: &mut Object, sel: Sel, drag_info: id) -> BOOL {
  let listener = unsafe { get_handler(this) };
  let paths = unsafe { collect_paths(drag_info) };

  let dl: NSPoint = unsafe { msg_send![drag_info, draggingLocation] };
  let frame: NSRect = unsafe { msg_send![this, frame] };
  let position = (dl.x as i32, (frame.size.height - dl.y) as i32);

  if !listener(FileDropEvent::Dropped { paths, position }) {
    // Reject the Wry file drop (invoke the OS default behaviour)
    OBJC_PERFORM_DRAG_OPERATION(this, sel, drag_info)
  } else {
    YES
  }
}

extern "C" fn dragging_exited(this: &mut Object, sel: Sel, drag_info: id) {
  let listener = unsafe { get_handler(this) };
  if !listener(FileDropEvent::Cancelled) {
    // Reject the Wry file drop (invoke the OS default behaviour)
    OBJC_DRAGGING_EXITED(this, sel, drag_info);
  }
}

pub(crate) unsafe fn add_file_drop_methods(decl: &mut ClassDecl) {
  decl.add_ivar::<*mut c_void>("FileDropHandler");

  decl.add_method(
    sel!(draggingUpdated:),
    dragging_updated as extern "C" fn(&mut Object, Sel, id) -> NSDragOperation,
  );

  decl.add_method(
    sel!(draggingEntered:),
    dragging_entered as extern "C" fn(&mut Object, Sel, id) -> NSDragOperation,
  );

  decl.add_method(
    sel!(performDragOperation:),
    perform_drag_operation as extern "C" fn(&mut Object, Sel, id) -> BOOL,
  );

  decl.add_method(
    sel!(draggingExited:),
    dragging_exited as extern "C" fn(&mut Object, Sel, id),
  );
}

#[cfg(test)]
mod suu_tests {
  //! Suu's patch: a drag naming the file names type with no list, or an empty one, reads no
  //! paths (it crashed Ableton Live, 2026-11-14).
  use super::*;
  use cocoa::base::nil;
  use objc::{declare::ClassDecl, runtime::Class};

  extern "C" fn pasteboard(this: &Object, _: Sel) -> id {
    unsafe { *this.get_ivar::<id>("pb") }
  }

  /// A stand-in for the drag: answers `draggingPasteboard` with `pb`.
  unsafe fn drag_with(pb: id) -> id {
    let cls = match Class::get("SuuFakeDrag") {
      Some(c) => c,
      None => {
        let mut decl = ClassDecl::new("SuuFakeDrag", class!(NSObject)).unwrap();
        decl.add_ivar::<id>("pb");
        decl.add_method(sel!(draggingPasteboard), pasteboard as extern "C" fn(&Object, Sel) -> id);
        decl.register()
      }
    };
    let obj: id = msg_send![cls, new];
    (*obj).set_ivar("pb", pb);
    obj
  }

  #[test]
  fn a_drag_without_a_file_list_reads_no_paths() {
    use cocoa::appkit::NSFilenamesPboardType;
    unsafe {
      let pb: id = msg_send![class!(NSPasteboard), pasteboardWithUniqueName];
      let types: id = msg_send![class!(NSArray), arrayWithObject: NSFilenamesPboardType];
      let _: i64 = msg_send![pb, declareTypes: types owner: nil];
      assert!(collect_paths(drag_with(pb)).is_empty());
      // an empty list (what Live's own drag gives): the fast iterator read a null pointer
      let empty: id = msg_send![class!(NSArray), array];
      let _: bool = msg_send![pb, setPropertyList: empty forType: NSFilenamesPboardType];
      assert!(collect_paths(drag_with(pb)).is_empty());
      // a real list still reads
      let path: id = msg_send![class!(NSString), stringWithUTF8String: b"/tmp/a.wav\0".as_ptr()];
      let list: id = msg_send![class!(NSArray), arrayWithObject: path];
      let _: bool = msg_send![pb, setPropertyList: list forType: NSFilenamesPboardType];
      assert_eq!(collect_paths(drag_with(pb)), vec![PathBuf::from("/tmp/a.wav")]);
    }
  }
}
