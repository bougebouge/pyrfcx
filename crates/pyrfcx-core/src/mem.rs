// Return free heap pages to the operating system.

/// Give the free pages of the C heap back to the operating system.
///
/// glibc keeps the pages of freed blocks in the process after the SDK
/// frees table rows, so the resident size stays at its peak. On Linux with
/// glibc this calls `malloc_trim(0)`, which returns whole free pages to the
/// kernel. It frees no live block and does not change how the allocator
/// works afterwards. On every other target it does nothing.
pub(crate) fn release_free_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            // glibc: int malloc_trim(size_t pad);
            fn malloc_trim(pad: usize) -> std::ffi::c_int;
        }
        // Safety: malloc_trim takes no pointer and touches only free heap
        // memory; glibc serialises it with every other allocator call. The
        // return value only says whether memory was released.
        unsafe {
            malloc_trim(0);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn release_free_memory_can_run_any_time() {
        let v = vec![0u8; 1 << 20];
        drop(v);
        super::release_free_memory();
        super::release_free_memory();
    }
}
