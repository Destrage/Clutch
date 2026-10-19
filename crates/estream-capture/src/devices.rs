//! What can be captured on this system, in a platform-neutral form (the first run uses it to
//! build a ready scene). Displays on Linux are chosen in the system's picker, so none are listed.

/// A display that display capture can show.
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    /// The id display capture's settings use.
    pub id: String,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

/// A camera that the camera source can open.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    /// The id the camera source's settings use.
    pub id: String,
    pub name: String,
}

pub fn displays() -> Vec<Display> {
    #[cfg(windows)]
    {
        crate::windows::enumerate::monitors()
            .into_iter()
            .map(|m| Display {
                id: m.id,
                width: m.width,
                height: m.height,
                primary: m.primary,
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

pub fn cameras() -> Vec<Camera> {
    #[cfg(windows)]
    {
        crate::windows::camera::devices()
            .into_iter()
            .map(|c| Camera {
                id: c.id,
                name: c.name,
            })
            .collect()
    }
    #[cfg(target_os = "linux")]
    {
        crate::linux::camera::devices()
            .into_iter()
            .map(|c| Camera {
                id: c.id,
                name: c.name,
            })
            .collect()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Vec::new()
    }
}
