use crate::{hid::ReportKind, input::MouseButton, passthrough::KeyUsage};

#[derive(Clone)]
pub struct InputReports {
    keys: [bool; 256],
    buttons: u8,
    wheel: [i32; 2],
}

impl Default for InputReports {
    fn default() -> Self {
        Self {
            keys: [false; 256],
            buttons: 0,
            wheel: [0; 2],
        }
    }
}

impl InputReports {
    pub fn key(&mut self, usage: u8, down: bool) -> Result<Vec<u8>, &'static str> {
        self.keys[usage as usize] = down;
        let mut report = vec![0; ReportKind::Keyboard.payload_len()];
        for modifier in 0..8 {
            if self.keys[0xe0 + modifier] {
                report[0] |= 1 << modifier;
            }
        }
        let mut slot = 2;
        for (usage, held) in self.keys.iter().enumerate().take(0xe0).skip(1) {
            if *held {
                if slot == report.len() {
                    return Err("Keyboard HID report capacity exceeded");
                }
                report[slot] = usage as u8;
                slot += 1;
            }
        }
        Ok(report)
    }

    pub fn button(&mut self, button: MouseButton, down: bool) -> Vec<u8> {
        let mask = 1 << button as u8;
        if down {
            self.buttons |= mask;
        } else {
            self.buttons &= !mask;
        }
        self.mouse_neutral()
    }

    pub fn mouse_neutral(&self) -> Vec<u8> {
        vec![self.buttons, 0, 0, 0, 0, 0, 0]
    }

    pub fn motion(&self, dx: i32, dy: i32) -> Result<Vec<u8>, &'static str> {
        let dx = i16::try_from(dx).map_err(|_| "Mouse X displacement exceeds HID range")?;
        let dy = i16::try_from(dy).map_err(|_| "Mouse Y displacement exceeds HID range")?;
        let mut report = self.mouse_neutral();
        report[1..3].copy_from_slice(&dx.to_le_bytes());
        report[3..5].copy_from_slice(&dy.to_le_bytes());
        Ok(report)
    }

    pub fn wheel(
        &mut self,
        vertical: i32,
        horizontal: i32,
    ) -> Result<Option<Vec<u8>>, &'static str> {
        self.wheel_with_direction(vertical, horizontal, false)
    }

    pub fn pointer(
        &mut self,
        dx: i32,
        dy: i32,
        vertical: i32,
        horizontal: i32,
        reverse: bool,
    ) -> Result<Vec<u8>, &'static str> {
        let mut report = self.motion(dx, dy)?;
        if let Some(wheel) = self.wheel_with_direction(vertical, horizontal, reverse)? {
            report[5..].copy_from_slice(&wheel[5..]);
        }
        Ok(report)
    }

    pub fn wheel_with_direction(
        &mut self,
        vertical: i32,
        horizontal: i32,
        reverse: bool,
    ) -> Result<Option<Vec<u8>>, &'static str> {
        let mut report = self.mouse_neutral();
        for (i, delta) in [vertical, horizontal].into_iter().enumerate() {
            let total = self.wheel[i]
                .checked_add(delta)
                .ok_or("Wheel displacement overflow")?;
            let steps =
                i8::try_from(total / 120).map_err(|_| "Wheel displacement exceeds HID range")?;
            self.wheel[i] = total % 120;
            report[5 + i] = if reverse {
                steps
                    .checked_neg()
                    .ok_or("Reversed wheel displacement exceeds HID range")?
            } else {
                steps
            } as u8;
        }
        Ok((report[5] != 0 || report[6] != 0).then_some(report))
    }
}

pub fn keyboard_usage(vk: u8, scan: u32, extended: bool) -> Option<KeyUsage> {
    let consumer = match vk {
        0xa6 => 0x224,
        0xa7 => 0x225,
        0xa8 => 0x227,
        0xa9 => 0x226,
        0xaa => 0x221,
        0xab => 0x22a,
        0xac => 0x223,
        0xad => 0xe2,
        0xae => 0xea,
        0xaf => 0xe9,
        0xb0 => 0xb5,
        0xb1 => 0xb6,
        0xb2 => 0xb7,
        0xb3 => 0xcd,
        0xb4 => 0x18a,
        0xb5 => 0x183,
        0xb6 => 0x194,
        0xb7 => 0x192,
        _ => 0,
    };
    if consumer != 0 {
        return Some(KeyUsage::Consumer(consumer));
    }
    let special = match vk {
        0x13 => Some(0x48),
        0x2c => Some(0x46),
        0x90 => Some(0x53),
        0x70..=0x7b => Some(0x3a + vk - 0x70),
        0x7c..=0x87 => Some(0x68 + vk - 0x7c),
        _ => None,
    };
    if let Some(usage) = special {
        return Some(KeyUsage::Keyboard(usage));
    }
    let usage = if extended {
        match scan & 0xff {
            0x1c => 0x58,
            0x1d => 0xe4,
            0x35 => 0x54,
            0x38 => 0xe6,
            0x47 => 0x4a,
            0x48 => 0x52,
            0x49 => 0x4b,
            0x4b => 0x50,
            0x4d => 0x4f,
            0x4f => 0x4d,
            0x50 => 0x51,
            0x51 => 0x4e,
            0x52 => 0x49,
            0x53 => 0x4c,
            0x5b => 0xe3,
            0x5c => 0xe7,
            0x5d => 0x65,
            _ => return None,
        }
    } else {
        match scan & 0xff {
            0x01 => 0x29,
            0x02..=0x0a => 0x1e + (scan as u8 - 2),
            0x0b => 0x27,
            0x0c => 0x2d,
            0x0d => 0x2e,
            0x0e => 0x2a,
            0x0f => 0x2b,
            0x10 => 0x14,
            0x11 => 0x1a,
            0x12 => 0x08,
            0x13 => 0x15,
            0x14 => 0x17,
            0x15 => 0x1c,
            0x16 => 0x18,
            0x17 => 0x0c,
            0x18 => 0x12,
            0x19 => 0x13,
            0x1a => 0x2f,
            0x1b => 0x30,
            0x1c => 0x28,
            0x1d => 0xe0,
            0x1e => 0x04,
            0x1f => 0x16,
            0x20 => 0x07,
            0x21 => 0x09,
            0x22 => 0x0a,
            0x23 => 0x0b,
            0x24 => 0x0d,
            0x25 => 0x0e,
            0x26 => 0x0f,
            0x27 => 0x33,
            0x28 => 0x34,
            0x29 => 0x35,
            0x2a => 0xe1,
            0x2b => 0x31,
            0x2c => 0x1d,
            0x2d => 0x1b,
            0x2e => 0x06,
            0x2f => 0x19,
            0x30 => 0x05,
            0x31 => 0x11,
            0x32 => 0x10,
            0x33 => 0x36,
            0x34 => 0x37,
            0x35 => 0x38,
            0x36 => 0xe5,
            0x37 => 0x55,
            0x38 => 0xe2,
            0x39 => 0x2c,
            0x3a => 0x39,
            0x3b..=0x44 => 0x3a + (scan as u8 - 0x3b),
            0x45 => 0x53,
            0x46 => 0x47,
            0x47 => 0x5f,
            0x48 => 0x60,
            0x49 => 0x61,
            0x4a => 0x56,
            0x4b => 0x5c,
            0x4c => 0x5d,
            0x4d => 0x5e,
            0x4e => 0x57,
            0x4f => 0x59,
            0x50 => 0x5a,
            0x51 => 0x5b,
            0x52 => 0x62,
            0x53 => 0x63,
            0x56 => 0x64,
            0x57 => 0x44,
            0x58 => 0x45,
            0x70 => 0x88,
            0x73 => 0x87,
            0x79 => 0x8a,
            0x7b => 0x8b,
            0x7d => 0x89,
            _ => return None,
        }
    };
    Some(KeyUsage::Keyboard(usage))
}
