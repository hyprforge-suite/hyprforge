pub mod displays;
pub mod keycapture;
pub mod layout_canvas;
pub mod shortcuts;
pub mod window_rules;

/// One lock for every test that repoints `$XDG_CONFIG_HOME`.
///
/// It has to be shared across modules, not one per module: the variable is
/// process-global, so two module-local locks don't exclude each other and a
/// `window_rules` test can retarget the config directory out from under a
/// `shortcuts` test mid-assertion. That produced a genuinely confusing
/// failure — a save reporting success while the file it wrote was nowhere to
/// be found, because it had landed in the other test's temp directory.
#[cfg(test)]
pub(crate) static CONFIG_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
