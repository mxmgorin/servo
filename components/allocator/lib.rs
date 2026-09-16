/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Selecting the default global allocator for Servo, and exposing common
//! allocator introspection APIs for memory profiling.

use std::os::raw::c_void;

#[cfg(not(feature = "allocation-tracking"))]
#[global_allocator]
static ALLOC: Allocator = Allocator;

#[cfg(feature = "allocation-tracking")]
#[global_allocator]
static ALLOC: crate::tracking::AccountingAlloc<Allocator> =
    crate::tracking::AccountingAlloc::with_allocator(Allocator);

#[cfg(feature = "allocation-tracking")]
mod tracking;

pub fn is_tracking_unmeasured() -> bool {
    cfg!(feature = "allocation-tracking")
}

pub fn dump_unmeasured(_writer: impl std::io::Write) {
    #[cfg(feature = "allocation-tracking")]
    ALLOC.dump_unmeasured_allocations(_writer);
}

pub fn disable_unmeasured_tracking() {
    #[cfg(feature = "allocation-tracking")]
    ALLOC.disable();
}

pub struct HeapReport {
    pub path: &'static str,
    pub size: Option<usize>,
}

pub use crate::platform::*;

// FIXME: temporary diagnostics for the memory report abort; remove before landing.

/// Pointers that [`usable_size`] was asked about and no heap of this process owns.
#[cfg(windows)]
pub fn take_foreign_pointers() -> Vec<String> {
    crate::platform::take_foreign_pointers()
}

#[cfg(not(windows))]
pub fn take_foreign_pointers() -> Vec<String> {
    Vec::new()
}

type EnclosingSizeFn = unsafe extern "C" fn(*const c_void) -> usize;

/// # Safety
/// No restrictions. The passed pointer is never dereferenced.
/// This function is only marked unsafe because the MallocSizeOfOps APIs
/// requires an unsafe function pointer.
#[cfg(feature = "allocation-tracking")]
unsafe extern "C" fn enclosing_size_impl(ptr: *const c_void) -> usize {
    let (adjusted, size) = crate::ALLOC.enclosing_size(ptr);
    if size != 0 {
        crate::ALLOC.note_allocation(adjusted, size);
    }
    size
}

#[expect(non_upper_case_globals)]
#[cfg(feature = "allocation-tracking")]
pub static enclosing_size: Option<EnclosingSizeFn> = Some(crate::enclosing_size_impl);

#[expect(non_upper_case_globals)]
#[cfg(not(feature = "allocation-tracking"))]
pub static enclosing_size: Option<EnclosingSizeFn> = None;

#[cfg(all(feature = "use-jemalloc", not(any(windows, target_env = "ohos"))))]
mod platform {
    use std::ffi::CStr;
    use std::mem::size_of_val;
    use std::os::raw::c_void;
    use std::ptr;

    use tikv_jemalloc_sys::mallctl;
    pub use tikv_jemallocator::Jemalloc as Allocator;

    pub fn heap_reports() -> Vec<crate::HeapReport> {
        vec![
            crate::HeapReport {
                path: "jemalloc-heap-allocated",
                size: jemalloc_stat(c"stats.allocated"),
            },
            crate::HeapReport {
                path: "jemalloc-heap-active",
                size: jemalloc_stat(c"stats.active"),
            },
            crate::HeapReport {
                path: "jemalloc-heap-mapped",
                size: jemalloc_stat(c"stats.mapped"),
            },
        ]
    }

    fn jemalloc_stat(value_name: &CStr) -> Option<usize> {
        // Before we request the measurement of interest, we first send an "epoch"
        // request. Without that jemalloc gives cached statistics(!) which can be
        // highly inaccurate.
        let epoch_c_name = c"epoch";
        let mut epoch: u64 = 0;
        let epoch_ptr = &raw mut epoch;
        let mut epoch_len = size_of_val(&epoch);

        let mut value: usize = 0;
        let value_ptr = &raw mut value;
        let mut value_len = size_of_val(&value);

        // Using the same values for the `old` and `new` parameters is enough
        // to get the statistics updated.
        let rv = unsafe {
            mallctl(
                epoch_c_name.as_ptr(),
                epoch_ptr.cast(),
                &mut epoch_len,
                epoch_ptr.cast(),
                epoch_len,
            )
        };
        if rv != 0 {
            return None;
        }

        let rv = unsafe {
            mallctl(
                value_name.as_ptr(),
                value_ptr.cast(),
                &mut value_len,
                ptr::null_mut(),
                0,
            )
        };
        if rv != 0 {
            return None;
        }

        Some(value)
    }

    /// Get the size of a heap block.
    ///
    /// # Safety
    ///
    /// Passing a non-heap allocated pointer to this function results in undefined behavior.
    pub unsafe extern "C" fn usable_size(ptr: *const c_void) -> usize {
        let size = unsafe { tikv_jemallocator::usable_size(ptr) };
        #[cfg(feature = "allocation-tracking")]
        crate::ALLOC.note_allocation(ptr, size);
        size
    }

    /// Memory allocation APIs compatible with libc
    pub mod libc_compat {
        pub use tikv_jemalloc_sys::{free, malloc, realloc};
    }
}

#[cfg(all(not(windows), any(target_env = "ohos", not(feature = "use-jemalloc"))))]
mod platform {
    pub use std::alloc::System as Allocator;
    use std::os::raw::c_void;

    /// Get the size of a heap block.
    ///
    /// # Safety
    ///
    /// Passing a non-heap allocated pointer to this function results in undefined behavior.
    pub unsafe extern "C" fn usable_size(ptr: *const c_void) -> usize {
        #[cfg(target_vendor = "apple")]
        unsafe {
            let size = libc::malloc_size(ptr);
            #[cfg(feature = "allocation-tracking")]
            crate::ALLOC.note_allocation(ptr, size);
            size
        }

        #[cfg(not(target_vendor = "apple"))]
        unsafe {
            let size = libc::malloc_usable_size(ptr as *mut _);
            #[cfg(feature = "allocation-tracking")]
            crate::ALLOC.note_allocation(ptr, size);
            size
        }
    }

    pub mod libc_compat {
        pub use libc::{free, malloc, realloc};
    }

    pub fn heap_reports() -> Vec<crate::HeapReport> {
        Vec::new()
    }
}

#[cfg(windows)]
mod platform {
    pub use std::alloc::System as Allocator;
    use std::os::raw::c_void;
    use std::sync::Mutex;

    use windows_sys::Win32::Foundation::{FALSE, HANDLE};
    use windows_sys::Win32::System::Memory::{
        GetProcessHeap, GetProcessHeaps, HeapSize, HeapValidate, MEM_COMMIT,
        MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    };

    /// Get the size of a heap block.
    ///
    /// # Safety
    ///
    /// No restrictions. Pointers that the process heap does not own measure as zero.
    pub unsafe extern "C" fn usable_size(ptr: *const c_void) -> usize {
        // FIXME: temporary diagnostics for the memory report abort; remove before landing.
        if ptr.is_null() || disabled() {
            return 0;
        }

        unsafe {
            let heap = GetProcessHeap();
            let Some(block) = process_heap_block(heap, ptr) else {
                record_foreign_pointer(ptr);
                return 0;
            };

            let size = HeapSize(heap, 0, block) as usize;
            #[cfg(feature = "allocation-tracking")]
            crate::ALLOC.note_allocation(block, size);
            size
        }
    }

    /// Blocks up to this alignment come straight from `HeapAlloc`; `System` over-allocates
    /// the rest.
    const MIN_ALIGN: usize = 2 * size_of::<usize>();

    /// The process heap block backing `ptr`, if there is one. `HeapSize` trusts the
    /// block header, so an unvalidated pointer fast-fails the whole process.
    unsafe fn process_heap_block(heap: HANDLE, ptr: *const c_void) -> Option<*const c_void> {
        if unsafe { HeapValidate(heap, 0, ptr) } != FALSE {
            return Some(ptr);
        }

        let base = unsafe { over_aligned_base(ptr) }?;
        (unsafe { HeapValidate(heap, 0, base) } != FALSE).then_some(base)
    }

    /// The base of the block `ptr` was carved out of: `System` over-allocates a block
    /// aligned beyond [`MIN_ALIGN`] and stores its base in the preceding word. A base too
    /// far from `ptr` is garbage, and `HeapValidate` faults on some of it.
    unsafe fn over_aligned_base(ptr: *const c_void) -> Option<*const c_void> {
        let alignment = 1usize << (ptr as usize).trailing_zeros();
        if alignment <= MIN_ALIGN {
            return None;
        }

        let base_slot = unsafe { (ptr as *const *const c_void).offset(-1) };
        if !is_readable(base_slot.cast()) {
            return None;
        }

        // `HeapAlloc` returned the base, the alignment moved `ptr` forward over the slot.
        let base = unsafe { *base_slot };
        let distance = (ptr as usize).checked_sub(base as usize)?;
        (distance >= size_of::<usize>() && distance < alignment + size_of::<usize>())
            .then_some(base)
    }

    /// Whether `ptr` may be dereferenced.
    fn is_readable(ptr: *const c_void) -> bool {
        let Some(info) = query(ptr) else {
            return false;
        };
        info.State == MEM_COMMIT && info.Protect & (PAGE_NOACCESS | PAGE_GUARD) == 0
    }

    fn query(ptr: *const c_void) -> Option<MEMORY_BASIC_INFORMATION> {
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let queried =
            unsafe { VirtualQuery(ptr, &mut info, size_of::<MEMORY_BASIC_INFORMATION>()) };
        (queried != 0).then_some(info)
    }

    // FIXME: temporary diagnostics for the memory report abort; remove before landing.

    /// Whether `SERVO_DISABLE_USABLE_SIZE` is set, which makes every measurement zero
    /// without touching the heap at all.
    fn disabled() -> bool {
        static DISABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *DISABLED.get_or_init(|| std::env::var_os("SERVO_DISABLE_USABLE_SIZE").is_some())
    }

    static FOREIGN_POINTERS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    const MAX_FOREIGN_POINTERS: usize = 32;
    const MAX_PROCESS_HEAPS: usize = 32;

    pub(crate) fn take_foreign_pointers() -> Vec<String> {
        std::mem::take(&mut FOREIGN_POINTERS.lock().unwrap())
    }

    fn record_foreign_pointer(ptr: *const c_void) {
        let Ok(mut recorded) = FOREIGN_POINTERS.lock() else {
            return;
        };
        if recorded.len() >= MAX_FOREIGN_POINTERS {
            return;
        }

        let owner = match owning_heap(ptr) {
            Some(heap) => format!("heap {heap:?}"),
            None => "no heap".to_owned(),
        };
        let Some(info) = query(ptr) else {
            recorded.push(format!("{ptr:p}: unmapped, {owner}"));
            return;
        };
        recorded.push(format!(
            "{ptr:p}: base={:p} region={:#x} state={:#x} type={:#x} protect={:#x}, {owner}",
            info.AllocationBase, info.RegionSize, info.State, info.Type, info.Protect,
        ));
    }

    /// The heap of this process that owns `ptr`, which tells a block of some other
    /// heap (a statically linked CRT, a DLL) apart from memory that is not heap at all.
    fn owning_heap(ptr: *const c_void) -> Option<HANDLE> {
        let mut heaps = [std::ptr::null_mut(); MAX_PROCESS_HEAPS];
        let count = unsafe { GetProcessHeaps(MAX_PROCESS_HEAPS as u32, heaps.as_mut_ptr()) };
        heaps
            .into_iter()
            .take(count as usize)
            .find(|heap| unsafe { HeapValidate(*heap, 0, ptr) } != FALSE)
    }

    pub fn heap_reports() -> Vec<crate::HeapReport> {
        Vec::new()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::os::raw::c_void;

    use windows_sys::Win32::Foundation::FALSE;
    use windows_sys::Win32::System::Memory::{
        GetProcessHeap, HeapValidate, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
        VirtualAlloc, VirtualFree,
    };

    static STATIC_DATA: [u8; 64] = [7; 64];

    const BLOCK_SIZE: usize = 64;

    #[test]
    fn heap_block_measures_its_size() {
        let block = Box::new([0u8; BLOCK_SIZE]);
        let size = unsafe { crate::usable_size(block.as_ptr().cast()) };
        assert!(size >= BLOCK_SIZE, "measured {size} bytes");
    }

    #[test]
    fn over_aligned_heap_block_measures_its_size() {
        #[repr(align(64))]
        struct OverAligned([u8; BLOCK_SIZE]);

        let block = Box::new(OverAligned([0; BLOCK_SIZE]));
        let size = unsafe { crate::usable_size(block.0.as_ptr().cast()) };
        assert!(size >= BLOCK_SIZE, "measured {size} bytes");
    }

    #[test]
    fn static_memory_measures_zero() {
        assert_eq!(unsafe { crate::usable_size(STATIC_DATA.as_ptr().cast()) }, 0);
    }

    #[test]
    fn stack_memory_measures_zero() {
        let on_stack = [0u8; BLOCK_SIZE];
        assert_eq!(unsafe { crate::usable_size(on_stack.as_ptr().cast()) }, 0);
    }

    #[test]
    fn reserved_pages_measure_zero() {
        let pages = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                BLOCK_SIZE,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        assert!(!pages.is_null(), "could not reserve pages");

        let size = unsafe { crate::usable_size(pages.cast_const()) };
        unsafe { VirtualFree(pages, 0, MEM_RELEASE) };
        assert_eq!(size, 0);
    }

    #[test]
    fn null_measures_zero() {
        assert_eq!(unsafe { crate::usable_size(std::ptr::null::<c_void>()) }, 0);
    }

    // Which pointers `HeapValidate` itself tolerates, measured apart from `usable_size`
    // so that an abort names the call that could not be made safe.

    #[test]
    fn heap_validate_tolerates_static_memory() {
        let heap = unsafe { GetProcessHeap() };
        assert_eq!(
            unsafe { HeapValidate(heap, 0, STATIC_DATA.as_ptr().cast()) },
            FALSE
        );
    }

    #[test]
    fn heap_validate_tolerates_stack_memory() {
        let on_stack = [0u8; BLOCK_SIZE];
        let heap = unsafe { GetProcessHeap() };
        assert_eq!(
            unsafe { HeapValidate(heap, 0, on_stack.as_ptr().cast()) },
            FALSE
        );
    }

    #[test]
    fn heap_validate_tolerates_an_unmapped_pointer() {
        let heap = unsafe { GetProcessHeap() };
        let unmapped = 0x1234_5678_9000usize as *const c_void;
        assert_eq!(unsafe { HeapValidate(heap, 0, unmapped) }, FALSE);
    }
}
