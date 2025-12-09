#![allow(missing_docs)]

use std::ffi::c_void;
use std::ffi::CString;
use std::mem;
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::ops::DerefMut;
use std::path::Path;
use std::ptr::null_mut;

use libc::c_char;

use crate::panic;
use crate::raw;
use crate::util::Binding;
use crate::Error;
use crate::IntoCString;
use crate::Oid;
use crate::Repository;

trait FilterInitialize<'a> {
    unsafe fn call(&self, filter: FilterInternal<'a>) -> Result<(), Error>;
}

trait FilterShutdown<'a> {
    unsafe fn call(&self, filter: FilterInternal<'a>) -> Result<(), Error>;
}

trait FilterCheck<'a> {
    unsafe fn call(
        &self,
        filter: FilterInternal<'a>,
        payload: *mut *mut c_void,
        src: *const raw::git_filter_source,
        attr_values: *const *const c_char,
    ) -> Result<bool, Error>;
}

trait FilterApply<'a> {
    unsafe fn call(
        &self,
        filter: FilterInternal<'a>,
        payload: *mut *mut c_void,
        to: *mut raw::git_buf,
        from: *const raw::git_buf,
        src: *const raw::git_filter_source,
    ) -> Result<bool, Error>;
}

trait FilterCleanup<'a> {
    unsafe fn call(
        &self,
        filter: FilterInternal<'a>,
        payload: *mut *mut c_void,
    ) -> Result<(), Error>;
}

struct FilterCallback<'a, P, F> {
    callback: F,
    _phantom: std::marker::PhantomData<&'a P>,
}

pub struct FilterBuf {
    raw: *mut raw::git_buf,
    data: Option<ManuallyDrop<Vec<u8>>>,
}

pub struct FilterPayload<P> {
    raw: *mut *mut c_void,
    data: Option<ManuallyDrop<Box<P>>>,
}

pub struct FilterRepository(ManuallyDrop<Repository>);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum FilterMode {
    Smudge = raw::GIT_FILTER_TO_WORKTREE,
    Clean = raw::GIT_FILTER_TO_ODB,
}

pub struct Filter<'f, P> {
    inner: *mut FilterRaw<'f>,
    _phantom: std::marker::PhantomData<&'f P>,
}

struct FilterInternal<'f> {
    inner: *mut FilterRaw<'f>,
}

#[repr(C)]
pub struct FilterRaw<'f> {
    raw: raw::git_filter,
    initialize: Option<Box<dyn FilterInitialize<'f> + 'f>>,
    shutdown: Option<Box<dyn FilterShutdown<'f> + 'f>>,
    check: Option<Box<dyn FilterCheck<'f> + 'f>>,
    apply: Option<Box<dyn FilterApply<'f> + 'f>>,
    cleanup: Option<Box<dyn FilterCleanup<'f> + 'f>>,
}

pub struct FilterSource {
    raw: *mut raw::git_filter_source,
}

impl Deref for FilterRepository {
    type Target = Repository;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl DerefMut for FilterRepository {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

impl<'f, P> Filter<'f, P> {
    pub fn new() -> Result<Self, Error> {
        crate::init();

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
            _phantom: std::marker::PhantomData,
        };

        unsafe {
            try_call!(raw::git_filter_init(
                filter.inner as *mut raw::git_filter,
                raw::GIT_FILTER_VERSION
            ));
        }

        unsafe {
            (*filter.inner).raw.cleanup = Some(on_cleanup);
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

impl<'f, P> Filter<'f, P> {
    pub fn on_init<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(Filter<'f, P>) -> Result<(), Error> + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.initialize = Some(on_init);
            inner.initialize = Some(Box::new(FilterCallback::<'f, P, F>::new(callback)));
        }
        self
    }

    pub fn on_shutdown<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(Filter<'f, P>) -> Result<(), Error> + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.shutdown = Some(on_shutdown);
            inner.shutdown = Some(Box::new(FilterCallback::<'f, P, F>::new(callback)));
        }
        self
    }

    pub fn on_check<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(Filter<'f, P>, FilterPayload<P>, FilterSource, Option<&str>) -> Result<bool, Error>
            + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.check = Some(on_check);
            inner.check = Some(Box::new(FilterCallback::<'f, P, F>::new(callback)));
        }
        self
    }

    pub fn on_apply<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(
                Filter<'f, P>,
                FilterPayload<P>,
                FilterBuf,
                FilterBuf,
                FilterSource,
            ) -> Result<bool, Error>
            + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.raw.stream = Some(on_stream);
            inner.apply = Some(Box::new(FilterCallback::<'f, P, F>::new(callback)));
        }
        self
    }

    pub fn on_cleanup<F>(&mut self, callback: F) -> &mut Self
    where
        F: Fn(Filter<'f, P>, Option<Box<P>>) -> Result<(), Error> + 'f,
    {
        if let Some(inner) = unsafe { self.inner.as_mut() } {
            inner.cleanup = Some(Box::new(FilterCallback::<'f, P, F>::new(callback)));
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

    pub fn register(mut self, name: &str, priority: i32) -> Result<(), Error> {
        unsafe {
            if (*self.inner).cleanup.is_none() {
                self.on_cleanup(|_, _| Ok(()));
            }

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

    pub fn repo(&self) -> FilterRepository {
        unsafe {
            let repo_ptr = call!(raw::git_filter_source_repo(self.raw));
            FilterRepository(ManuallyDrop::new(Repository::from_raw(repo_ptr)))
        }
    }
}

impl<P> FilterPayload<P> {
    pub fn inner(&self) -> Option<&Box<P>> {
        match &self.data {
            Some(data) => Some(&*data.deref()),
            None => None,
        }
    }

    pub fn inner_mut(&mut self) -> Option<&mut Box<P>> {
        match &mut self.data {
            Some(data) => Some(&mut *data.deref_mut()),
            None => None,
        }
    }

    pub fn replace(&mut self, data: P) {
        _ = self.take();

        let ptr = Box::into_raw(Box::new(data));

        self.data = Some(ManuallyDrop::new(unsafe { Box::from_raw(ptr) }));
        unsafe {
            *self.raw = ptr as *mut c_void;
        }
    }

    pub fn take(&mut self) -> Option<Box<P>> {
        let data = unsafe {
            match &mut self.data {
                Some(data) => Some(ManuallyDrop::take(data)),
                None => None,
            }
        };

        data
    }
}

impl<'f> FilterInternal<'f> {
    fn cast<P>(&self) -> Filter<'f, P> {
        unsafe { Filter::from_raw(self.inner as *mut raw::git_filter) }
    }
}

impl Drop for FilterBuf {
    fn drop(&mut self) {
        self.sync();
    }
}

impl<P> Binding for FilterPayload<P> {
    type Raw = *mut *mut c_void;

    unsafe fn from_raw(raw: *mut *mut c_void) -> FilterPayload<P> {
        if (*raw).is_null() {
            FilterPayload { data: None, raw }
        } else {
            FilterPayload {
                raw,
                data: Some(ManuallyDrop::new(Box::from_raw(*raw as *mut _))),
            }
        }
    }

    fn raw(&self) -> Self::Raw {
        self.raw
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

impl<'f, P> Binding for Filter<'f, P> {
    type Raw = *mut raw::git_filter;

    unsafe fn from_raw(raw: *mut raw::git_filter) -> Filter<'f, P> {
        Filter {
            inner: raw as *mut FilterRaw<'f>,
            _phantom: std::marker::PhantomData,
        }
    }

    fn raw(&self) -> Self::Raw {
        &self.inner as *const _ as *mut _
    }
}

impl<'f> Binding for FilterInternal<'f> {
    type Raw = *mut FilterRaw<'f>;

    unsafe fn from_raw(raw: *mut FilterRaw<'f>) -> FilterInternal<'f> {
        FilterInternal { inner: raw }
    }

    fn raw(&self) -> Self::Raw {
        self.inner
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

impl<'a, P, F> FilterCallback<'a, P, F> {
    fn new(callback: F) -> Self {
        Self {
            callback,
            _phantom: std::marker::PhantomData,
        }
    }
}

extern "C" fn on_init(filter: *mut raw::git_filter) -> i32 {
    let ok = panic::wrap(|| unsafe {
        let filter = FilterInternal::from_raw(filter as *mut _);

        if let Some(ref initialize) = (*filter.inner).initialize {
            initialize.call(filter)
        } else {
            Ok(())
        }
    });

    match ok {
        Some(Ok(())) => 0,
        Some(Err(e)) => e.raw_code(),
        None => -1,
    }
}

impl<'a, P, F> FilterInitialize<'a> for FilterCallback<'a, P, F>
where
    F: Fn(Filter<'a, P>) -> Result<(), Error> + 'a,
{
    unsafe fn call(&self, filter: FilterInternal<'a>) -> Result<(), Error> {
        (self.callback)(filter.cast::<P>())
    }
}

extern "C" fn on_shutdown(filter: *mut raw::git_filter) -> i32 {
    let ok = panic::wrap(|| unsafe {
        let filter = FilterInternal::from_raw(filter as *mut _);

        if let Some(ref shutdown) = (*filter.inner).shutdown {
            shutdown.call(filter)
        } else {
            Ok(())
        }
    });

    match ok {
        Some(Ok(())) => 0,
        Some(Err(e)) => e.raw_code(),
        None => -1,
    }
}

impl<'a, P, F> FilterShutdown<'a> for FilterCallback<'a, P, F>
where
    F: Fn(Filter<'a, P>) -> Result<(), Error> + 'a,
{
    unsafe fn call(&self, filter: FilterInternal<'a>) -> Result<(), Error> {
        (self.callback)(filter.cast::<P>())
    }
}

extern "C" fn on_check(
    filter: *mut raw::git_filter,
    payload: *mut *mut libc::c_void,
    src: *const raw::git_filter_source,
    attr_values: *const *const i8,
) -> i32 {
    let ok = panic::wrap(|| unsafe {
        let filter = FilterInternal::from_raw(filter as *mut _);

        if let Some(ref check) = (*filter.inner).check {
            check.call(
                filter,
                payload,
                src as *const raw::git_filter_source,
                attr_values,
            )
        } else {
            Ok(false)
        }
    });

    match ok {
        Some(Ok(true)) => 0,
        Some(Ok(false)) => raw::GIT_PASSTHROUGH,
        Some(Err(e)) => e.raw_code(),
        None => -1,
    }
}

impl<'a, P, F> FilterCheck<'a> for FilterCallback<'a, P, F>
where
    F: Fn(Filter<'a, P>, FilterPayload<P>, FilterSource, Option<&str>) -> Result<bool, Error> + 'a,
{
    unsafe fn call(
        &self,
        filter: FilterInternal<'a>,
        payload: *mut *mut c_void,
        src: *const raw::git_filter_source,
        attr_values: *const *const c_char,
    ) -> Result<bool, Error> {
        (self.callback)(
            filter.cast::<P>(),
            FilterPayload::<P>::from_raw(payload),
            FilterSource::from_raw(src as *mut _),
            if attr_values.is_null() {
                None
            } else {
                str::from_utf8(*attr_values.cast()).ok()
            },
        )
    }
}

extern "C" fn on_apply(
    filter: *mut raw::git_filter,
    payload: *mut *mut libc::c_void,
    to: *mut raw::git_buf,
    from: *const raw::git_buf,
    src: *const raw::git_filter_source,
) -> i32 {
    let ok = panic::wrap(|| unsafe {
        let filter = FilterInternal::from_raw(filter as *mut _);

        if let Some(ref apply) = (*filter.inner).apply {
            apply.call(filter, payload, to, from, src)
        } else {
            Ok(true)
        }
    });

    match ok {
        Some(Ok(true)) => 0,
        Some(Ok(false)) => raw::GIT_PASSTHROUGH,
        Some(Err(e)) => e.raw_code(),
        None => -1,
    }
}

impl<'a, P, F> FilterApply<'a> for FilterCallback<'a, P, F>
where
    F: Fn(
            Filter<'a, P>,
            FilterPayload<P>,
            FilterBuf,
            FilterBuf,
            FilterSource,
        ) -> Result<bool, Error>
        + 'a,
{
    unsafe fn call(
        &self,
        filter: FilterInternal<'a>,
        payload: *mut *mut c_void,
        to: *mut raw::git_buf,
        from: *const raw::git_buf,
        src: *const raw::git_filter_source,
    ) -> Result<bool, Error> {
        (self.callback)(
            filter.cast::<P>(),
            FilterPayload::<P>::from_raw(payload),
            FilterBuf::from_raw(to),
            FilterBuf::from_raw(from as *mut _),
            FilterSource::from_raw(src as *mut _),
        )
    }
}

extern "C" fn on_cleanup(filter: *mut raw::git_filter, mut payload: *mut libc::c_void) {
    panic::wrap(move || unsafe {
        let filter = FilterInternal::from_raw(filter as *mut _);
        if let Some(ref cleanup) = (*filter.inner).cleanup {
            cleanup
                .call(filter, &mut payload as *mut *mut c_void)
                .is_ok()
        } else {
            true
        }
    });
}

impl<'a, P, F> FilterCleanup<'a> for FilterCallback<'a, P, F>
where
    F: Fn(Filter<'a, P>, Option<Box<P>>) -> Result<(), Error> + 'a,
{
    unsafe fn call(
        &self,
        filter: FilterInternal<'a>,
        payload: *mut *mut c_void,
    ) -> Result<(), Error> {
        (self.callback)(
            filter.cast::<P>(),
            FilterPayload::<P>::from_raw(payload).take(),
        )
    }
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
