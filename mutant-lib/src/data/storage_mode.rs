pub(crate) const LIGHTEST_SCRATCHPAD_SIZE: usize = 512 * 1024;
pub(crate) const LIGHT_SCRATCHPAD_SIZE: usize = 1 * 1024 * 1024;
pub(crate) const MEDIUM_SCRATCHPAD_SIZE: usize = 2 * 1024 * 1024;
pub(crate) const HEAVY_SCRATCHPAD_SIZE: usize = 3 * 1024 * 1024;
pub(crate) const HEAVIEST_SCRATCHPAD_SIZE: usize = 4 * 1024 * 1024;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum StorageMode {
    /// 0.5 MB per scratchpad
    Lightest,
    /// 1 MB per scratchpad
    Light,
    /// 2 MB per scratchpad
    Medium,
    /// 3 MB per scratchpad
    Heavy,
    /// 4 MB per scratchpad
    Heaviest,
}

impl StorageMode {
    pub fn scratchpad_size(&self) -> usize {
        match self {
            StorageMode::Lightest => LIGHTEST_SCRATCHPAD_SIZE,
            StorageMode::Light => LIGHT_SCRATCHPAD_SIZE,
            StorageMode::Medium => MEDIUM_SCRATCHPAD_SIZE,
            StorageMode::Heavy => HEAVY_SCRATCHPAD_SIZE,
            StorageMode::Heaviest => HEAVIEST_SCRATCHPAD_SIZE,
        }
    }
}
