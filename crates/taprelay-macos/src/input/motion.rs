use super::Point;

/// Some remote input sources provide only screen coordinates. Retain their
/// last position so a zero CG mouse delta does not turn all movement into
/// empty HID reports.
#[derive(Default)]
pub(super) struct Motion {
    previous: Option<(i64, Point)>,
    fraction: [f64; 2],
    previous_relative: bool,
}

impl Motion {
    pub(super) fn relative(&mut self, source: i64, position: Point, delta: [i32; 2]) -> [i32; 2] {
        let previous = self.previous.replace((source, position));
        let previous_relative = std::mem::replace(&mut self.previous_relative, delta != [0, 0]);
        if delta != [0, 0] {
            self.fraction = [0.; 2];
            return delta;
        }
        let Some((previous_source, previous)) = previous else {
            return [0, 0];
        };
        if previous_source != source {
            self.fraction = [0.; 2];
            return [0, 0];
        }
        // After a relative sample, the app may have recentered the cursor.
        // That position jump is not input from an absolute source.
        if previous_relative {
            return [0, 0];
        }
        let movement = [position.x - previous.x, position.y - previous.y];
        let mut delta = [0; 2];
        for (axis, movement) in movement.into_iter().enumerate() {
            if !movement.is_finite() {
                self.fraction = [0.; 2];
                return [0, 0];
            }
            self.fraction[axis] += movement;
            delta[axis] = self.fraction[axis].trunc() as i32;
            self.fraction[axis] -= f64::from(delta[axis]);
        }
        delta
    }
}
