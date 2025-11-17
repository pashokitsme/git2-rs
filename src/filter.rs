#![allow(missing_docs)]

use std::ffi::CString;
use std::mem;
use std::mem::ManuallyDrop;
use std::path::Path;
use std::ptr::null_mut;

use crate::panic;
use crate::raw;
use crate::util::Binding;
use crate::Error;
use crate::IntoCString;
use crate::Oid;

pub type FilterInitialize<'a> = dyn Fn(Filter<'a>) -> Result<(), Error> + 'a;
pub type FilterShutdown<'a> = dyn Fn(Filter<'a>) -> Result<(), Error> + 'a;
pub type FilterCheck<'a> =
    dyn Fn(Filter<'a>, FilterSource, Option<&str>) -> Result<bool, Error> + 'a;
pub type FilterApply<'a> =
    dyn Fn(Filter<'a>, FilterBuf, FilterBuf, FilterSource) -> Result<(), Error> + 'a;
pub type FilterCleanup<'a> = dyn Fn(Filter<'a>) -> Result<(), Error> + 'a;

pub struct FilterBuf {
    raw: *mut raw::git_buf,
    data: Option<ManuallyDrop<Vec<u8>>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum FilterMode {
    Smudge = raw::GIT_FILTER_TO_WORKTREE,
    Clean = raw::GIT_FILTER_TO_ODB,
}

pub struct Filter<'f> {
    inner: *mut FilterRaw<'f>,
}

pub struct FilterSource {
    raw: *mut raw::git_filter_source,
}

#[repr(C)]
pub struct FilterRaw<'f> {
    raw: raw::git_filter,
    initialize: Option<Box<FilterInitialize<'f>>>,
    shutdown: Option<Box<FilterShutdown<'f>>>,
    check: Option<Box<FilterCheck<'f>>>,
    apply: Option<Box<FilterApply<'f>>>,
    cleanup: Option<Box<FilterCleanup<'f>>>,
}

impl<'f> Filter<'f> {
    pub fn new() -> Result<Self, Error> {
        let inner = Box::new(FilterRaw {
            raw: unsafe { mem::zeroed() },
            initialize: None,
            shutdown: None,
            check: None,
            apply: None,
            cleanup: None,
        });

        let filter = Self {
            inner: Box::into_raw(inner),
        };

        unsafe {
            try_call!(raw::git_filter_init(
                filter.inner as *mut raw::git_filter,
                raw::GIT_FILTER_VERSION
            ));
        }

        Ok(filter)
    }
}

impl FilterBuf {
    pub fn as_bytes(&self) -> &[u8] {
        self.data
            .as_ref()
            .map(|data| data.as_slice())
            .unwrap_or_default()
    }

    pub fn as_allocated_vec(&mut self) -> &mut Vec<u8> {
        if self.data.is_none() {
            self.data = Some(ManuallyDrop::new(Vec::new()));
        }

        self.data.as_mut().unwrap()
    }

    pub fn sync(&mut self) {
        if let Some(data) = &self.data {
            unsafe {
                (*self.raw).ptr = data.as_ptr() as *mut i8;
                (*self.raw).size = data.len() as usize;
            }
        }
    }
}

impl<'f> Filter<'f> {
    pub fn on_init<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(Filter<'f>) -> Result<(), Error> + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.initialize = Some(on_init);
            inner.initialize = Some(Box::new(callback) as Box<FilterInitialize<'f>>);
        }
        self
    }

    pub fn on_shutdown<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(Filter<'f>) -> Result<(), Error> + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.shutdown = Some(on_shutdown);
            inner.shutdown = Some(Box::new(callback) as Box<FilterShutdown<'f>>);
        }
        self
    }

    pub fn on_check<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(Filter<'f>, FilterSource, Option<&str>) -> Result<bool, Error> + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.check = Some(on_check);
            inner.check = Some(Box::new(callback) as Box<FilterCheck<'f>>);
        }
        self
    }

    pub fn on_apply<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(Filter<'f>, FilterBuf, FilterBuf, FilterSource) -> Result<(), Error> + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.stream = Some(on_stream);
            inner.apply = Some(Box::new(callback) as Box<FilterApply<'f>>);
        }
        self
    }

    pub fn on_cleanup<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(Filter<'f>) -> Result<(), Error> + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.cleanup = Some(on_cleanup);
            inner.cleanup = Some(Box::new(callback) as Box<FilterCleanup<'f>>);
        }
        self
    }

    pub fn attributes(&mut self, attrs: &str) -> Result<&mut Self, Error> {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            if !inner.raw.attributes.is_null() {
                drop(unsafe { CString::from_raw(inner.raw.attributes as *mut i8) });
            }
            inner.raw.attributes = attrs.into_c_string()?.into_raw()
        }
        Ok(self)
    }

    pub fn register(self, name: &str, priority: i32) -> Result<(), Error> {
        unsafe {
            try_call!(raw::git_filter_register(
                name.into_c_string()?.into_raw(),
                self.inner as *mut raw::git_filter,
                priority
            ));
        }

        Ok(())
    }
}

impl FilterSource {
    pub fn id(&self) -> Option<Oid> {
        unsafe {
            match call!(raw::git_filter_source_id(self.raw)) {
                oid_raw if !oid_raw.is_null() => Some(Oid::from_raw(oid_raw)),
                _ => None,
            }
        }
    }

    pub fn mode(&self) -> FilterMode {
        unsafe {
            match call!(raw::git_filter_source_mode(self.raw)) {
                mode => FilterMode::from_raw(mode),
            }
        }
    }

    pub fn path_bytes(&self) -> Option<&[u8]> {
        static FOO: () = ();
        let path = unsafe { call!(raw::git_filter_source_path(self.raw)) };
        unsafe { crate::opt_bytes(&FOO, path) }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path_bytes().map(crate::util::bytes2path)
    }

    pub fn filemode(&self) -> Option<u16> {
        unsafe {
            match call!(raw::git_filter_source_filemode(self.raw)) {
                filemode if filemode != 0 => Some(filemode),
                _ => None,
            }
        }
    }
}

impl Drop for FilterBuf {
    fn drop(&mut self) {
        self.sync();
    }
}

impl Binding for FilterBuf {
    type Raw = *mut raw::git_buf;

    unsafe fn from_raw(raw: *mut raw::git_buf) -> FilterBuf {
        let data = if (*raw).ptr.is_null() {
            None
        } else {
            Some(ManuallyDrop::new(Vec::from_raw_parts(
                (*raw).ptr as *mut u8,
                (*raw).size,
                (*raw).size,
            )))
        };

        FilterBuf { raw, data }
    }

    fn raw(&self) -> Self::Raw {
        self.raw
    }
}

impl<'f> Binding for Filter<'f> {
    type Raw = *mut raw::git_filter;

    unsafe fn from_raw(raw: *mut raw::git_filter) -> Filter<'f> {
        Filter {
            inner: raw as *mut FilterRaw<'f>,
        }
    }

    fn raw(&self) -> Self::Raw {
        &self.inner as *const _ as *mut _
    }
}

impl Binding for FilterSource {
    type Raw = *mut raw::git_filter_source;

    unsafe fn from_raw(raw: *mut raw::git_filter_source) -> FilterSource {
        FilterSource { raw }
    }

    fn raw(&self) -> *mut raw::git_filter_source {
        self.raw
    }
}

impl Binding for FilterMode {
    type Raw = raw::git_filter_mode_t;

    unsafe fn from_raw(raw: raw::git_filter_mode_t) -> FilterMode {
        match raw {
            raw::GIT_FILTER_TO_WORKTREE => FilterMode::Smudge,
            raw::GIT_FILTER_TO_ODB => FilterMode::Clean,
            _ => unreachable!(),
        }
    }

    fn raw(&self) -> raw::git_filter_mode_t {
        match self {
            FilterMode::Smudge => raw::GIT_FILTER_TO_WORKTREE,
            FilterMode::Clean => raw::GIT_FILTER_TO_ODB,
        }
    }
}

extern "C" fn on_init(filter: *mut raw::git_filter) -> i32 {
    let ok = panic::wrap(|| unsafe {
        let filter = Filter::from_raw(filter);

        if let Some(ref initialize) = (*filter.inner).initialize {
            initialize(filter).is_ok()
        } else {
            true
        }
    });

    if ok == Some(true) {
        0
    } else {
        -1
    }
}

extern "C" fn on_apply(
    filter: *mut raw::git_filter,
    _payload: *mut *mut libc::c_void,
    to: *mut raw::git_buf,
    from: *const raw::git_buf,
    src: *const raw::git_filter_source,
) -> i32 {
    let ok = panic::wrap(|| unsafe {
        let filter = Filter::from_raw(filter);

        let to = FilterBuf::from_raw(to);
        let from = FilterBuf::from_raw(from as *mut _);

        let src = FilterSource::from_raw(src as *mut _);

        if let Some(ref apply) = (*filter.inner).apply {
            apply(filter, to, from, src).is_ok()
        } else {
            true
        }
    });

    if ok == Some(true) {
        0
    } else {
        -1
    }
}

extern "C" fn on_cleanup(filter: *mut raw::git_filter, _payload: *mut libc::c_void) {
    panic::wrap(|| unsafe {
        let filter = Filter::from_raw(filter);

        if let Some(ref initialize) = (*filter.inner).cleanup {
            initialize(filter).is_ok()
        } else {
            true
        }
    });
}

extern "C" fn on_stream(
    out: *mut *mut raw::git_writestream,
    filter: *mut raw::git_filter,
    payload: *mut *mut libc::c_void,
    src: *const raw::git_filter_source,
    next: *mut raw::git_writestream,
) -> i32 {
    unsafe {
        call!(raw::git_filter_buffered_stream_new(
            out,
            filter,
            on_apply,
            null_mut::<raw::git_buf>(),
            payload,
            src,
            next
        ))
    }
}

extern "C" fn on_check(
    filter: *mut raw::git_filter,
    _payload: *mut *mut libc::c_void,
    src: *const raw::git_filter_source,
    attr_values: *const *const i8,
) -> i32 {
    let ok = panic::wrap(|| unsafe {
        let filter = Filter::from_raw(filter);

        if let Some(ref check) = (*filter.inner).check {
            let attrs = if attr_values.is_null() {
                None
            } else {
                str::from_utf8(*attr_values.cast()).ok()
            };

            check(filter, FilterSource::from_raw(src as *mut _), attrs).ok()
        } else {
            Some(false)
        }
    })
    .flatten();

    match ok {
        Some(true) => 0,
        Some(false) => raw::GIT_PASSTHROUGH,
        None => -1,
    }
}

extern "C" fn on_shutdown(filter: *mut raw::git_filter) -> i32 {
    let ok = panic::wrap(|| unsafe {
        let filter = Filter::from_raw(filter);

        if let Some(ref shutdown) = (*filter.inner).shutdown {
            shutdown(filter).is_ok()
        } else {
            true
        }
    });

    if ok == Some(true) {
        0
    } else {
        -1
    }
}
