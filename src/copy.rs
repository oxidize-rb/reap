use read_process_memory::{copy_address, CopyAddress, Pid, ProcessHandle, TryIntoProcessHandle};

// Adapted from rbspy
#[inline]
pub fn copy_struct<U, T>(addr: usize, source: &T) -> Result<U, std::io::Error>
where
    T: CopyAddress,
{
    let result = copy_address(addr, std::mem::size_of::<U>(), source)?;
    let s: U = unsafe { std::ptr::read(result.as_ptr() as *const _) };
    Ok(s)
}

// Adapted from rbspy
#[inline]
pub fn copy_vec<U, T>(addr: usize, length: usize, source: &T) -> Result<Vec<U>, std::io::Error>
where
    T: CopyAddress,
{
    let mut vec = copy_address(addr, length * std::mem::size_of::<U>(), source)?;
    let capacity = vec.capacity() as usize / std::mem::size_of::<U>() as usize;
    let ptr = vec.as_mut_ptr() as *mut U;
    std::mem::forget(vec);
    unsafe { Ok(Vec::from_raw_parts(ptr, capacity, capacity)) }
}
