#![allow(missing_docs)]

use std::ffi::CString;
use std::mem;
use std::path::Path;

use crate::panic;
use crate::raw;
use crate::util::Binding;
use crate::Buf;
use crate::Error;
use crate::IntoCString;
use crate::Oid;

pub type FilterInitialize<'a> = dyn Fn(Filter<'a>) -> Result<(), Error> + 'a;
pub type FilterShutdown<'a> = dyn Fn(Filter<'a>) -> Result<(), Error> + 'a;
pub type FilterCheck<'a> = dyn Fn(Filter<'a>, FilterSource, Option<&str>) -> Result<(), Error> + 'a;
pub type FilterApply<'a> = dyn Fn(Filter<'a>, Buf, Buf, FilterSource) -> Result<(), Error> + 'a;
pub type FilterStream<'a> = dyn Fn(Filter<'a>) -> Result<(), Error> + 'a;
pub type FilterCleanup<'a> = dyn Fn(Filter<'a>) -> Result<(), Error> + 'a;

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
    stream: Option<Box<FilterStream<'f>>>,
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
            stream: None,
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
        F: Fn(Filter<'f>, FilterSource, Option<&str>) -> Result<(), Error> + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.check = Some(on_check);
            inner.check = Some(Box::new(callback) as Box<FilterCheck<'f>>);
        }
        self
    }

    pub fn on_apply<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(Filter<'f>, Buf, Buf, FilterSource) -> Result<(), Error> + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.apply = Some(on_apply);
            inner.apply = Some(Box::new(callback) as Box<FilterApply<'f>>);
        }
        self
    }

    pub fn on_stream<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(Filter<'f>) -> Result<(), Error> + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.stream = Some(on_stream);
            inner.stream = Some(Box::new(callback) as Box<FilterStream<'f>>);
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

// impl<'f> Drop for Filter<'f> {
//     fn drop(&mut self) {
//         println!("Dropping Filter");
//     }
// }

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
        let to = Buf::from_raw(to);
        let from = Buf::from_raw(from as *mut _);
        let src = FilterSource::from_raw(src as *mut _);

        if let Some(ref apply) = (*filter.inner).apply {
            apply(filter, from, to, src).is_ok()
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
    _streams: *mut *mut raw::git_writestream,
    _filter: *mut raw::git_filter,
    _payload: *mut *mut libc::c_void,
    _src: *const raw::git_filter_source,
    _next: *mut raw::git_writestream,
) -> i32 {
    println!("on_stream");
    0
    // return git_filter_buffered_stream_new(out, filter, crlf_apply, NULL, payload, src, next);
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

            check(filter, FilterSource::from_raw(src as *mut _), attrs).is_ok()
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
