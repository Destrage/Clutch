//! Camera formats, shared by the platform camera sources. Subtypes use the same names on
//! every system (`NV12`, `YUY2`, `MJPG`), so a saved format key reads the same everywhere.

#[derive(Clone, Debug, PartialEq)]
pub struct CameraFormat {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub subtype: String,
}

impl CameraFormat {
    pub fn fps(&self) -> f64 {
        self.fps_num as f64 / self.fps_den.max(1) as f64
    }

    pub fn key(&self) -> String {
        format!(
            "{}x{}@{}/{}:{}",
            self.width, self.height, self.fps_num, self.fps_den, self.subtype
        )
    }

    pub fn label(&self) -> String {
        format!(
            "{} × {} · {:.4} fps · {}",
            self.width,
            self.height,
            self.fps(),
            self.subtype
        )
        .replace(".0000 fps", " fps")
    }
}

/// Picks the default format: smooth motion (24 fps or more) when the camera has it, then the
/// highest resolution up to 1080p, then the highest fps up to 60.
pub fn best_format(list: &[CameraFormat]) -> Option<CameraFormat> {
    list.iter()
        .filter(|f| f.height <= 1080 && f.fps() <= 61.0)
        .max_by_key(|f| {
            (
                f.fps() >= 23.9,
                f.width * f.height,
                (f.fps() * 1000.0) as u64,
                // Uncompressed formats cost no decoding.
                match f.subtype.as_str() {
                    "NV12" => 2,
                    "YUY2" => 1,
                    _ => 0,
                },
            )
        })
        .or_else(|| list.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(w: u32, h: u32, fps: u32, sub: &str) -> CameraFormat {
        CameraFormat {
            width: w,
            height: h,
            fps_num: fps,
            fps_den: 1,
            subtype: sub.into(),
        }
    }

    #[test]
    fn best_format_prefers_1080p60() {
        let list = vec![
            f(3840, 2160, 30, "MJPG"),
            f(1920, 1080, 30, "YUY2"),
            f(1920, 1080, 60, "MJPG"),
            f(1280, 720, 60, "NV12"),
        ];
        assert_eq!(best_format(&list).unwrap(), f(1920, 1080, 60, "MJPG"));
        assert_eq!(
            best_format(&[f(3840, 2160, 30, "MJPG")]).unwrap().width,
            3840
        );
        assert_eq!(f(1920, 1080, 60, "MJPG").key(), "1920x1080@60/1:MJPG");
        // A bigger picture at a choppy rate loses to smooth 720p.
        assert_eq!(
            best_format(&[f(1280, 800, 10, "YUY2"), f(1280, 720, 30, "MJPG")]).unwrap(),
            f(1280, 720, 30, "MJPG")
        );
    }
}
