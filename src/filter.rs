use std::ffi::CString;
use std::marker;
use std::path::Path;
use std::ptr;

use bitflags::bitflags;

use crate::util::Binding;
use crate::{raw, Blob, Buf, Error, IntoCString, Oid, Repository};

/// Filter mode determines the direction of the filter operation.
///
/// Filters are applied in one of two directions: smudging - which is
/// exporting a file from the Git object database to the working directory,
/// and cleaning - which is importing a file from the working directory to
/// the Git object database.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FilterMode {
    /// Export a file from the Git object database to the working directory
    ToWorktree = raw::GIT_FILTER_TO_WORKTREE as isize,
    /// Import a file from the working directory to the Git object database
    ToOdb = raw::GIT_FILTER_TO_ODB as isize,
}

impl FilterMode {
    /// Alias for `ToWorktree`
    pub const SMUDGE: FilterMode = FilterMode::ToWorktree;
    /// Alias for `ToOdb`
    pub const CLEAN: FilterMode = FilterMode::ToOdb;

    #[allow(dead_code)]
    fn from_raw(raw: raw::git_filter_mode_t) -> FilterMode {
        match raw {
            raw::GIT_FILTER_TO_WORKTREE => FilterMode::ToWorktree,
            raw::GIT_FILTER_TO_ODB => FilterMode::ToOdb,
            _ => FilterMode::ToWorktree,
        }
    }

    fn to_raw(self) -> raw::git_filter_mode_t {
        match self {
            FilterMode::ToWorktree => raw::GIT_FILTER_TO_WORKTREE,
            FilterMode::ToOdb => raw::GIT_FILTER_TO_ODB,
        }
    }
}

bitflags! {
    /// Filter option flags.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
    pub struct FilterFlag: u32 {
        /// Default filter options
        const DEFAULT = raw::GIT_FILTER_DEFAULT as u32;
        /// Don't error for `safecrlf` violations, allow them to continue
        const ALLOW_UNSAFE = raw::GIT_FILTER_ALLOW_UNSAFE as u32;
        /// Don't load `/etc/gitattributes` (or the system equivalent)
        const NO_SYSTEM_ATTRIBUTES = raw::GIT_FILTER_NO_SYSTEM_ATTRIBUTES as u32;
        /// Load attributes from `.gitattributes` in the root of HEAD
        const ATTRIBUTES_FROM_HEAD = raw::GIT_FILTER_ATTRIBUTES_FROM_HEAD as u32;
        /// Load attributes from `.gitattributes` in a given commit
        const ATTRIBUTES_FROM_COMMIT = raw::GIT_FILTER_ATTRIBUTES_FROM_COMMIT as u32;
    }
}

/// Filtering options
#[derive(Clone, Debug)]
pub struct FilterOptions {
    flags: FilterFlag,
    attr_commit_id: Option<Oid>,
}

impl Default for FilterOptions {
    fn default() -> Self {
        FilterOptions {
            flags: FilterFlag::DEFAULT,
            attr_commit_id: None,
        }
    }
}

impl FilterOptions {
    /// Creates a new set of filter options with default values.
    pub fn new() -> FilterOptions {
        FilterOptions::default()
    }

    /// Set filter flags
    pub fn flags(&mut self, flags: FilterFlag) -> &mut Self {
        self.flags = flags;
        self
    }

    /// Set the commit to load attributes from when `ATTRIBUTES_FROM_COMMIT` is specified
    pub fn attr_commit_id(&mut self, oid: Option<Oid>) -> &mut Self {
        self.attr_commit_id = oid;
        self
    }

    fn raw(&mut self) -> raw::git_filter_options {
        let mut opts = raw::git_filter_options {
            version: raw::GIT_FILTER_OPTIONS_VERSION,
            flags: self.flags.bits() as raw::git_filter_flag_t,
            commit_id: ptr::null_mut(),
            attr_commit_id: ptr::null_mut(),
        };

        if let Some(ref oid) = self.attr_commit_id {
            opts.attr_commit_id = oid.raw() as *mut _;
        }

        opts
    }
}

/// A list of filters to be applied to a file/blob.
///
/// This represents a list of filters to be applied to a file / blob. You
/// can build the list with one call, apply it with another, and dispose it
/// with a third. In typical usage, there are not many occasions where a
/// `FilterList` is needed directly since the library will generally
/// handle conversions for you, but it can be convenient to be able to
/// build and apply the list sometimes.
pub struct FilterList<'repo> {
    raw: *mut raw::git_filter_list,
    _marker: marker::PhantomData<&'repo Repository>,
}

impl<'repo> FilterList<'repo> {
    /// Load the filter list for a given path.
    ///
    /// This will return `Ok(None)` if no filters are requested for the given file.
    ///
    /// # Arguments
    ///
    /// * `repo` - Repository object that contains `path`
    /// * `blob` - The blob to which the filter will be applied (if known), can be `None`
    /// * `path` - Relative path of the file to be filtered
    /// * `mode` - Filtering direction (WT->ODB or ODB->WT)
    /// * `flags` - Combination of filter flags
    pub fn load(
        repo: &'repo Repository,
        blob: Option<&Blob<'repo>>,
        path: &Path,
        mode: FilterMode,
        flags: FilterFlag,
    ) -> Result<Option<FilterList<'repo>>, Error> {
        crate::init();
        let path = path.into_c_string()?;
        let blob_ptr = blob.map(|b| b.raw()).unwrap_or(ptr::null_mut());
        let mut filters = ptr::null_mut();
        unsafe {
            try_call!(raw::git_filter_list_load(
                &mut filters,
                repo.raw(),
                blob_ptr,
                path.as_ptr(),
                mode.to_raw(),
                flags.bits()
            ));
            if filters.is_null() {
                Ok(None)
            } else {
                Ok(Some(Binding::from_raw(filters)))
            }
        }
    }

    /// Load the filter list for a given path with extended options.
    ///
    /// This will return `Ok(None)` if no filters are requested for the given file.
    ///
    /// # Arguments
    ///
    /// * `repo` - Repository object that contains `path`
    /// * `blob` - The blob to which the filter will be applied (if known), can be `None`
    /// * `path` - Relative path of the file to be filtered
    /// * `mode` - Filtering direction (WT->ODB or ODB->WT)
    /// * `opts` - Filter options
    pub fn load_ext(
        repo: &'repo Repository,
        blob: Option<&Blob<'repo>>,
        path: &Path,
        mode: FilterMode,
        opts: &mut FilterOptions,
    ) -> Result<Option<FilterList<'repo>>, Error> {
        crate::init();
        let path = path.into_c_string()?;
        let blob_ptr = blob.map(|b| b.raw()).unwrap_or(ptr::null_mut());
        let mut filters = ptr::null_mut();
        let mut raw_opts = opts.raw();
        unsafe {
            try_call!(raw::git_filter_list_load_ext(
                &mut filters,
                repo.raw(),
                blob_ptr,
                path.as_ptr(),
                mode.to_raw(),
                &mut raw_opts
            ));
            if filters.is_null() {
                Ok(None)
            } else {
                Ok(Some(Binding::from_raw(filters)))
            }
        }
    }

    /// Query the filter list to see if a given filter (by name) will run.
    ///
    /// The built-in filters "crlf" and "ident" can be queried, otherwise this
    /// is the name of the filter specified by the filter attribute.
    ///
    /// This will return `false` if the given filter is not in the list, or `true` if
    /// the filter will be applied.
    pub fn contains(&self, name: &str) -> bool {
        let name = match CString::new(name) {
            Ok(s) => s,
            Err(_) => return false,
        };
        unsafe { raw::git_filter_list_contains(self.raw, name.as_ptr()) == 1 }
    }

    /// Apply filter list to a data buffer.
    ///
    /// # Arguments
    ///
    /// * `input` - Buffer containing the data to filter
    pub fn apply_to_buffer(&self, input: &[u8]) -> Result<Buf, Error> {
        crate::init();
        let out = Buf::new();
        unsafe {
            try_call!(raw::git_filter_list_apply_to_buffer(
                out.raw(),
                self.raw,
                input.as_ptr() as *const _,
                input.len()
            ));
            Ok(out)
        }
    }

    /// Apply a filter list to the contents of a file on disk.
    ///
    /// # Arguments
    ///
    /// * `repo` - The repository in which to perform the filtering
    /// * `path` - The path of the file to filter, a relative path will be taken as relative to the workdir
    pub fn apply_to_file(&self, repo: &Repository, path: &Path) -> Result<Buf, Error> {
        crate::init();
        let path = path.into_c_string()?;
        let out = Buf::new();
        unsafe {
            try_call!(raw::git_filter_list_apply_to_file(
                out.raw(),
                self.raw,
                repo.raw(),
                path.as_ptr()
            ));
            Ok(out)
        }
    }

    /// Apply a filter list to the contents of a blob.
    ///
    /// # Arguments
    ///
    /// * `blob` - The blob to filter
    pub fn apply_to_blob(&self, blob: &Blob<'repo>) -> Result<Buf, Error> {
        crate::init();
        let out = Buf::new();
        unsafe {
            try_call!(raw::git_filter_list_apply_to_blob(
                out.raw(),
                self.raw,
                blob.raw()
            ));
            Ok(out)
        }
    }

    /// Get the number of filters in this list.
    pub fn len(&self) -> usize {
        unsafe { raw::git_filter_list_length(self.raw) as usize }
    }

    /// Check if the filter list is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<'repo> Binding for FilterList<'repo> {
    type Raw = *mut raw::git_filter_list;

    unsafe fn from_raw(raw: *mut raw::git_filter_list) -> FilterList<'repo> {
        FilterList {
            raw,
            _marker: marker::PhantomData,
        }
    }

    fn raw(&self) -> *mut raw::git_filter_list {
        self.raw
    }
}

impl<'repo> Drop for FilterList<'repo> {
    fn drop(&mut self) {
        unsafe {
            raw::git_filter_list_free(self.raw);
        }
    }
}

// Extension methods for Repository
impl Repository {
    /// Load the filter list for a given path.
    ///
    /// This will return `Ok(None)` if no filters are requested for the given file.
    ///
    /// # Arguments
    ///
    /// * `blob` - The blob to which the filter will be applied (if known), can be `None`
    /// * `path` - Relative path of the file to be filtered
    /// * `mode` - Filtering direction (WT->ODB or ODB->WT)
    /// * `flags` - Combination of filter flags
    pub fn filter_list_load(
        &self,
        blob: Option<&Blob<'_>>,
        path: &Path,
        mode: FilterMode,
        flags: FilterFlag,
    ) -> Result<Option<FilterList<'_>>, Error> {
        crate::init();
        let path = path.into_c_string()?;
        let blob_ptr = blob.map(|b| b.raw()).unwrap_or(ptr::null_mut());
        let mut filters = ptr::null_mut();
        unsafe {
            try_call!(raw::git_filter_list_load(
                &mut filters,
                self.raw(),
                blob_ptr,
                path.as_ptr(),
                mode.to_raw(),
                flags.bits()
            ));
            if filters.is_null() {
                Ok(None)
            } else {
                Ok(Some(Binding::from_raw(filters)))
            }
        }
    }

    /// Load the filter list for a given path with extended options.
    ///
    /// This will return `Ok(None)` if no filters are requested for the given file.
    ///
    /// # Arguments
    ///
    /// * `blob` - The blob to which the filter will be applied (if known), can be `None`
    /// * `path` - Relative path of the file to be filtered
    /// * `mode` - Filtering direction (WT->ODB or ODB->WT)
    /// * `opts` - Filter options
    pub fn filter_list_load_ext(
        &self,
        blob: Option<&Blob<'_>>,
        path: &Path,
        mode: FilterMode,
        opts: &mut FilterOptions,
    ) -> Result<Option<FilterList<'_>>, Error> {
        crate::init();
        let path = path.into_c_string()?;
        let blob_ptr = blob.map(|b| b.raw()).unwrap_or(ptr::null_mut());
        let mut filters = ptr::null_mut();
        let mut raw_opts = opts.raw();
        unsafe {
            try_call!(raw::git_filter_list_load_ext(
                &mut filters,
                self.raw(),
                blob_ptr,
                path.as_ptr(),
                mode.to_raw(),
                &mut raw_opts
            ));
            if filters.is_null() {
                Ok(None)
            } else {
                Ok(Some(Binding::from_raw(filters)))
            }
        }
    }
}
