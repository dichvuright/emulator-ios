//! Interactive Apple iOS 16.7.16 (20H392) Operating System & Stock Apps Engine (`sweet-gpu`).
//! Provides full touch/click/drag/scroll/keyboard interactivity and rich vector UI rendering
//! for Settings (and all its subpages), Phone, Calculator, Safari, Messages, Camera, Photos,
//! Clock, Notes, Reminders, Music, Weather, Files, App Store, Control Center, Spotlight,
//! Calendar, Mail, Maps, Health, Wallet, Home, Stocks, Books, Podcasts, TV, and FaceTime.

use crate::font::{
    draw_sdf_numeral, draw_text_centered, draw_text_right, draw_text_scaled, draw_text_semibold,
    measure_sdf_numerals, measure_text_px,
};
use crate::raster::{
    blur_rounded_rect, draw_line, draw_ring, fill_circle, fill_rect, fill_rounded_rect,
    fill_rounded_rect_gradient, fill_triangle, render_squircle_icon, rgb, rgba,
};
use crate::springboard::{
    draw_home_indicator, draw_ios16_wallpaper, draw_iphone_x_status_bar, draw_stock_icon_canvas,
};
use crate::IosSimState;
use std::fs;
use std::path::PathBuf;

pub const HOME_GRID_APPS: [&str; 20] = [
    "FaceTime",
    "Calendar",
    "Photos",
    "Camera",
    "Mail",
    "Clock",
    "Maps",
    "Weather",
    "Reminders",
    "Notes",
    "Stocks",
    "Books",
    "App Store",
    "Podcasts",
    "TV",
    "Health",
    "Home",
    "Wallet",
    "Settings",
    "Files",
];

pub const DOCK_APPS: [&str; 4] = ["Phone", "Safari", "Messages", "Music"];

pub const MUSIC_TRACKS: [(&str, &str, &str, u32, u32); 4] = [
    (
        "As It Was",
        "Harry Styles",
        "Harry's House — Lossless",
        0xFFE6_3946,
        0xFF1D_3557,
    ),
    (
        "Anti-Hero",
        "Taylor Swift",
        "Midnights (3am Edition)",
        0xFF45_7B9D,
        0xFF1D_2D44,
    ),
    (
        "Blinding Lights",
        "The Weeknd",
        "After Hours — Dolby Atmos",
        0xFFD9_0429,
        0xFF2B_2D42,
    ),
    (
        "Waiting For You",
        "MONO",
        "22 (Album) — Apple Music VN",
        0xFF72_09B7,
        0xFF3A_0CA3,
    ),
];

pub const WEATHER_CITIES: [(&str, i32, &str, i32, i32, &str); 3] = [
    (
        "Ho Chi Minh City",
        31,
        "Partly Cloudy",
        34,
        26,
        "Warm tropical breeze around 14 km/h. UV Index: 7.",
    ),
    (
        "Ha Noi",
        26,
        "Mostly Clear",
        28,
        21,
        "Pleasant autumn air across Hoan Kiem Lake. Humidity 64%.",
    ),
    (
        "Cupertino",
        21,
        "Sunny",
        24,
        13,
        "Clear California skies over Apple Park. Visibility 16 km.",
    ),
];

impl IosSimState {
    /// Initializes realistic default state for all iOS 16 apps and settings on first use.
    pub fn ensure_initialized(&mut self) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        self.wifi_enabled = true;
        if self.wifi_network.is_empty() {
            self.wifi_network = "SweetNet-5G".to_string();
        }
        self.bluetooth_enabled = true;
        self.cellular_enabled = true;
        self.true_tone = true;
        if self.brightness == 0 {
            self.brightness = 100;
        }
        if self.volume == 0 {
            self.volume = 70;
        }
        self.phone_tab = 3; // Keypad
        if self.calc_display.is_empty() {
            self.calc_display = "0".to_string();
        }
        self.calc_replace_next = true;
        self.camera_mode = 1; // PHOTO
        if self.photos_count == 0 {
            self.photos_count = 6;
        }
        self.photos_favorites = 0b000101;
        self.alarms_enabled = 0b0101;
        self.timer_remaining_ticks = 15 * 60 * 60;
        self.home_devices_on = 0b0111;
        if self.calendar_selected_day == 0 {
            self.calendar_selected_day = 20;
        }
        if self.phone_recents.is_empty() {
            self.phone_recents = vec![
                ("Khanh (iPhone X)".to_string(), "Mobile • 9:38 AM".to_string()),
                ("Apple Support".to_string(), "Cupertino • Yesterday".to_string()),
                ("Viettel 198".to_string(), "Carrier • Yesterday".to_string()),
                ("Team ios-sweet".to_string(), "FaceTime Audio • Mon".to_string()),
            ];
        }
        if self.messages_threads.is_empty() {
            self.messages_threads = vec![
                (
                    "Apple".to_string(),
                    "9:40 AM".to_string(),
                    vec![
                        (
                            false,
                            "Welcome to iPhone X on iOS 16.7.16 (20H392)!".to_string(),
                        ),
                        (
                            false,
                            "All stock apps, Settings, Phone, Safari, and Camera are ready."
                                .to_string(),
                        ),
                        (true, "Awesome, testing interactive UI now!".to_string()),
                    ],
                ),
                (
                    "Khanh (iPhone X)".to_string(),
                    "9:35 AM".to_string(),
                    vec![
                        (
                            false,
                            "Anh dang test cac chuc nang tren may ao iOS 16.".to_string(),
                        ),
                        (
                            true,
                            "Da fix full icon vector + tat ca ung dung deu bam duoc anh nhe!"
                                .to_string(),
                        ),
                    ],
                ),
                (
                    "Viettel".to_string(),
                    "Yesterday".to_string(),
                    vec![(
                        false,
                        "Quy khach dang su dung goi 5G Viettel toc do cao (10GB/ngay)."
                            .to_string(),
                    )],
                ),
                (
                    "Team ios-sweet".to_string(),
                    "Monday".to_string(),
                    vec![
                        (
                            false,
                            "XNU 22.6.0 + Apple D22AP rootfs mounted on /dev/vda.".to_string(),
                        ),
                        (true, "SpringBoard 60 FPS running smoothly!".to_string()),
                    ],
                ),
            ];
        }
        if self.notes_items.is_empty() {
            self.notes_items = vec![
                (
                    "iOS 16.7.16 Bring-Up Plan".to_string(),
                    "1. Boot XNU 22.6.0 on sweet-virt\n2. Mount 16GB ios16-root-20H392.img\n3. Interactive SpringBoard & Apps"
                        .to_string(),
                ),
                (
                    "Shopping & Devices".to_string(),
                    "- iPhone X (iPhone10,3 / D22AP)\n- Redmi Note 10 Pro (SM7150)\n- AirPods Pro (2nd Gen)"
                        .to_string(),
                ),
                (
                    "Quick Scratchpad".to_string(),
                    "Tap any key on the keyboard below to type into this note!".to_string(),
                ),
            ];
        }
        if self.reminders_items.is_empty() {
            self.reminders_items = vec![
                "Test Settings Wi-Fi, Dark Mode & About".to_string(),
                "Dial a number in Phone & test Call UI".to_string(),
                "Calculate expressions in Calculator".to_string(),
                "Browse websites in Safari & send iMessage".to_string(),
                "Take photos in Camera & view in Photos".to_string(),
            ];
            self.reminders_done = 0b00001;
        }
        self.lock_anim = if self.locked { 1.0 } else { 0.0 };
        self.prev_locked = self.locked;
        self.home_flyin = 1.0;
        self.nav_anim = 1.0;
        self.app_anim = if self.active_app.is_some() { 1.0 } else { 0.0 };
        if let Some(ref a) = self.active_app {
            self.anim_app_name = Some(a.clone());
            self.anim_icon_rect = self.icon_rect_for_app(a);
        }
    }

    /// Returns the exact `(x, y, w, h)` icon rectangle in `360x800` viewport coordinates for an app.
    pub fn icon_rect_for_app(&self, app_name: &str) -> (f32, f32, f32, f32) {
        if app_name.eq_ignore_ascii_case("Control Center") {
            return (268.0, 8.0, 76.0, 24.0);
        }
        let icon_size = 54.0f32;
        for (i, &dock) in DOCK_APPS.iter().enumerate() {
            if app_name.eq_ignore_ascii_case(dock) {
                let dock_margin = 12.0f32;
                let dock_w = 360.0 - 2.0 * dock_margin;
                let cell_w = dock_w / 4.0;
                let cx = dock_margin + (i as f32) * cell_w + cell_w * 0.5;
                let ix = cx - icon_size * 0.5;
                let iy = (800.0 - 104.0) + (80.0 - icon_size) * 0.5;
                return (ix, iy, icon_size, icon_size);
            }
        }
        let has_row6 = self
            .installed_apps
            .iter()
            .any(|a| !a.display_name.trim().is_empty() && !a.display_name.eq_ignore_ascii_case("Sweet 3D Bench"));
        let grid_top = if has_row6 { 48.0f32 } else { 54.0f32 };
        let row_h = if has_row6 {
            icon_size + 24.0
        } else {
            icon_size + 30.0
        };
        let grid_margin_x = 18.0f32;
        let cell_w = (360.0 - 2.0 * grid_margin_x) / 4.0;

        for (i, &stock) in HOME_GRID_APPS.iter().enumerate() {
            if app_name.eq_ignore_ascii_case(stock) {
                let col = (i % 4) as f32;
                let row = (i / 4) as f32;
                let cx = grid_margin_x + col * cell_w + cell_w * 0.5;
                let ix = cx - icon_size * 0.5;
                let iy = grid_top + row * row_h;
                return (ix, iy, icon_size, icon_size);
            }
        }
        for (col_idx, app) in self.installed_apps.iter().take(4).enumerate() {
            if app.display_name.eq_ignore_ascii_case(app_name)
                || app.bundle_id.eq_ignore_ascii_case(app_name)
            {
                let col = col_idx as f32;
                let cx = grid_margin_x + col * cell_w + cell_w * 0.5;
                let ix = cx - icon_size * 0.5;
                let iy = grid_top + 5.0 * row_h;
                return (ix, iy, icon_size, icon_size);
            }
        }
        (153.0, 340.0, icon_size, icon_size)
    }

    /// Advances per-frame animations and timers (~60-120 Hz).
    pub fn tick(&mut self) {
        self.ensure_initialized();
        self.frame_tick = self.frame_tick.wrapping_add(1);
        if self.stopwatch_running {
            self.stopwatch_elapsed_ticks = self.stopwatch_elapsed_ticks.wrapping_add(1);
        }
        if self.timer_running {
            if self.timer_remaining_ticks > 0 {
                self.timer_remaining_ticks -= 1;
            } else {
                self.timer_running = false;
            }
        }
        if self.music_playing {
            self.music_progress_ticks = (self.music_progress_ticks + 1) % (180 * 60);
        }

        // 1. Lock Screen curtain & staggered Home Screen icon fly-in spring animation
        if self.prev_locked && !self.locked {
            self.home_flyin = 0.0;
        }
        self.prev_locked = self.locked;
        if !self.locked && self.home_flyin < 1.0 {
            self.home_flyin = (self.home_flyin + 0.042).min(1.0);
        }

        if self.locked && self.touch_down && self.drag_start_vy >= 520 {
            let drag_up = (self.drag_start_vy - (self.touch_y / 3)).max(0) as f32;
            self.lock_anim = (1.0 - drag_up / 340.0).clamp(0.06, 1.0);
            self.lock_vel = 0.0;
        } else {
            let lock_target = if self.locked { 1.0f32 } else { 0.0f32 };
            let force = (lock_target - self.lock_anim) * 0.24;
            self.lock_vel = (self.lock_vel + force) * 0.72;
            self.lock_anim = (self.lock_anim + self.lock_vel).clamp(0.0, 1.0);
            if (self.lock_anim - lock_target).abs() < 0.004 && self.lock_vel.abs() < 0.004 {
                self.lock_anim = lock_target;
                self.lock_vel = 0.0;
            }
        }

        // 2. App Launch / Home-Swipe Card Morph Spring Animation
        if let Some(ref cur_app) = self.active_app {
            if self.anim_app_name.as_deref() != Some(cur_app.as_str()) {
                self.anim_app_name = Some(cur_app.clone());
                self.anim_icon_rect = self.icon_rect_for_app(cur_app);
                self.app_anim = 0.04;
                self.app_vel = 0.085;
            }
        }
        if self.active_app.is_some() && self.touch_down && self.edge_swipe_active {
            let drag_up = (792 - (self.touch_y / 3)).max(0) as f32;
            self.app_anim = (1.0 - (drag_up / 340.0).min(0.76)).clamp(0.24, 1.0);
            self.app_vel = 0.0;
        } else {
            let app_target = if self.active_app.is_some() { 1.0f32 } else { 0.0f32 };
            let force = (app_target - self.app_anim) * 0.24;
            self.app_vel = (self.app_vel + force) * 0.70;
            self.app_anim = (self.app_anim + self.app_vel).clamp(0.0, 1.0);
            if (self.app_anim - app_target).abs() < 0.005 && self.app_vel.abs() < 0.005 {
                self.app_anim = app_target;
                self.app_vel = 0.0;
                if app_target == 0.0 {
                    self.anim_app_name = None;
                }
            }
        }

        // 3. Settings Subpage Push/Pop Horizontal Slide Animation
        if self.settings_page != self.last_seen_settings_page {
            let is_deeper = !self.settings_page.is_empty()
                && (self.last_seen_settings_page.is_empty()
                    || (self.last_seen_settings_page == "General"
                        && matches!(
                            self.settings_page.as_str(),
                            "About" | "SoftwareUpdate" | "Storage"
                        )));
            self.prev_settings_page = self.last_seen_settings_page.clone();
            self.last_seen_settings_page = self.settings_page.clone();
            self.nav_dir = if is_deeper { 1 } else { -1 };
            self.nav_anim = 0.06;
        }
        if self.nav_anim < 1.0 {
            self.nav_anim = (self.nav_anim + (1.0 - self.nav_anim) * 0.26).min(1.0);
            if self.nav_anim > 0.992 {
                self.nav_anim = 1.0;
            }
        }

        // 4. Inertial Vertical Scroll Momentum
        if !self.touch_down && self.scroll_vel.abs() > 0.45 {
            self.scroll_y = (self.scroll_y + self.scroll_vel.round() as i32).clamp(0, 340);
            self.scroll_vel *= 0.88;
            if self.scroll_vel.abs() <= 0.45 {
                self.scroll_vel = 0.0;
            }
        }
    }

    /// Navigates one level back or returns to the Home Screen.
    pub fn handle_back_gesture(&mut self) -> Option<String> {
        self.ensure_initialized();
        if self.locked {
            self.locked = false;
            return Some("Unlocked iPhone X to Home Screen".to_string());
        }
        if self.spotlight_open {
            self.spotlight_open = false;
            return Some("Closed Spotlight Search".to_string());
        }
        let app = self.active_app.clone()?;
        self.scroll_y = 0;

        if app.eq_ignore_ascii_case("Settings") && !self.settings_page.is_empty() {
            if matches!(
                self.settings_page.as_str(),
                "About" | "SoftwareUpdate" | "Storage"
            ) {
                self.settings_page = "General".to_string();
                return Some("Settings -> General".to_string());
            } else {
                self.settings_page.clear();
                return Some("Settings -> Main".to_string());
            }
        }
        if app.eq_ignore_ascii_case("Messages") && self.messages_active_chat.is_some() {
            self.messages_active_chat = None;
            return Some("Messages -> Conversations".to_string());
        }
        if app.eq_ignore_ascii_case("Photos") && self.photos_selected.is_some() {
            self.photos_selected = None;
            return Some("Photos -> Grid".to_string());
        }
        if app.eq_ignore_ascii_case("Notes") && self.notes_selected.is_some() {
            self.notes_selected = None;
            return Some("Notes -> All iCloud".to_string());
        }
        if app.eq_ignore_ascii_case("Mail") && self.mail_selected_idx.is_some() {
            self.mail_selected_idx = None;
            return Some("Mail -> Inbox".to_string());
        }
        if app.eq_ignore_ascii_case("Files") && !self.files_current_dir.is_empty() {
            self.files_current_dir.clear();
            return Some("Files -> On My iPhone".to_string());
        }
        if app.eq_ignore_ascii_case("Safari") {
            if self.safari_editing_url {
                self.safari_editing_url = false;
                return Some("Closed Safari Keyboard".to_string());
            }
            if !self.safari_url.is_empty() {
                self.safari_url = self.safari_history.pop().unwrap_or_default();
                return Some("Safari -> Back".to_string());
            }
        }
        self.active_app = None;
        Some(format!("Closed {} -> Home Screen", app))
    }

    /// Scrolls the current view vertically.
    pub fn handle_scroll(&mut self, wheel_delta: i32) {
        self.ensure_initialized();
        let step = if wheel_delta > 0 { -36 } else { 36 };
        self.scroll_y = (self.scroll_y + step).clamp(0, 340);
    }

    /// Handles dragging sliders or scrolling lists in `360x800` viewport coordinates.
    pub fn handle_drag(&mut self, vx: i32, vy: i32, _prev_vx: i32, prev_vy: i32) {
        self.ensure_initialized();
        let app = match &self.active_app {
            Some(a) => a.as_str(),
            None => return,
        };
        if app.eq_ignore_ascii_case("Control Center") {
            if (187..=336).contains(&vx) && (227..=376).contains(&vy) {
                let pct = ((376 - vy) * 100 / 149).clamp(10, 100) as u8;
                if vx < 261 {
                    self.brightness = pct;
                } else {
                    self.volume = pct;
                }
                return;
            }
        }
        if app.eq_ignore_ascii_case("Settings") {
            if self.settings_page == "Display"
                && (32..=328).contains(&vx)
                && (235..=285).contains(&vy)
            {
                self.brightness = (((vx - 44) * 100 / 272).clamp(15, 100)) as u8;
                return;
            }
            if self.settings_page == "Sounds"
                && (32..=328).contains(&vx)
                && (130..=180).contains(&vy)
            {
                self.volume = (((vx - 44) * 100 / 272).clamp(0, 100)) as u8;
                return;
            }
        }
        let dy = prev_vy - vy;
        if dy.abs() >= 1 {
            self.scroll_y = (self.scroll_y + dy).clamp(0, 340);
            self.scroll_vel = (dy as f32) * 0.9;
        }
    }

    /// Handles PC keyboard or on-screen keyboard character input.
    pub fn handle_key(&mut self, ch: char) -> Option<String> {
        self.ensure_initialized();
        if self.spotlight_open {
            match ch {
                '\x08' => {
                    self.spotlight_query.pop();
                }
                '\r' | '\n' => {
                    let q = self.spotlight_query.trim().to_lowercase();
                    for &app in HOME_GRID_APPS.iter().chain(DOCK_APPS.iter()) {
                        if app.to_lowercase().contains(&q) {
                            self.spotlight_open = false;
                            self.active_app = Some(app.to_string());
                            return Some(format!("Spotlight launched {}", app));
                        }
                    }
                    self.spotlight_open = false;
                }
                c if !c.is_control() && self.spotlight_query.len() < 28 => {
                    self.spotlight_query.push(c);
                }
                _ => {}
            }
            return Some(format!("Spotlight: '{}'", self.spotlight_query));
        }

        let app = self.active_app.clone()?;
        if app.eq_ignore_ascii_case("Calculator") {
            return match ch {
                '0'..='9' => {
                    self.calc_press(&ch.to_string());
                    Some(format!("Calculator: {}", self.calc_display))
                }
                '.' => {
                    self.calc_press(".");
                    Some(format!("Calculator: {}", self.calc_display))
                }
                '+' => {
                    self.calc_press("+");
                    Some("Calculator: +".to_string())
                }
                '-' => {
                    self.calc_press("-");
                    Some("Calculator: -".to_string())
                }
                '*' | 'x' | 'X' => {
                    self.calc_press("×");
                    Some("Calculator: ×".to_string())
                }
                '/' => {
                    self.calc_press("÷");
                    Some("Calculator: ÷".to_string())
                }
                '=' | '\r' | '\n' => {
                    self.calc_press("=");
                    Some(format!("Calculator = {}", self.calc_display))
                }
                'c' | 'C' | '\x1b' => {
                    self.calc_press("AC");
                    Some("Calculator: AC".to_string())
                }
                '\x08' => {
                    if self.calc_display.len() > 1 {
                        self.calc_display.pop();
                    } else {
                        self.calc_display = "0".to_string();
                        self.calc_replace_next = true;
                    }
                    Some(format!("Calculator: {}", self.calc_display))
                }
                _ => None,
            };
        }

        if app.eq_ignore_ascii_case("Phone") && self.phone_tab == 3 && !self.phone_in_call {
            match ch {
                '0'..='9' | '*' | '#' | '+' => {
                    if self.phone_dial_buf.len() < 16 {
                        self.phone_dial_buf.push(ch);
                    }
                    return Some(format!("Dialed {}", self.phone_dial_buf));
                }
                '\x08' => {
                    self.phone_dial_buf.pop();
                    return Some(format!("Dial buffer: {}", self.phone_dial_buf));
                }
                '\r' | '\n' => {
                    self.start_phone_call(None);
                    return Some(format!("Calling {}...", self.phone_call_target));
                }
                _ => {}
            }
        }

        if app.eq_ignore_ascii_case("Safari") && self.safari_editing_url {
            match ch {
                '\x08' => {
                    self.safari_input_buf.pop();
                }
                '\r' | '\n' => {
                    self.commit_safari_url();
                    return Some(format!("Safari navigated to {}", self.safari_url));
                }
                c if !c.is_control() && self.safari_input_buf.len() < 42 => {
                    self.safari_input_buf.push(c);
                }
                _ => {}
            }
            return Some(format!("Safari URL: {}", self.safari_input_buf));
        }

        if app.eq_ignore_ascii_case("Messages") && self.messages_active_chat.is_some() {
            match ch {
                '\x08' => {
                    self.messages_input_buf.pop();
                }
                '\r' | '\n' => {
                    return self.send_active_imessage();
                }
                c if !c.is_control() && self.messages_input_buf.len() < 48 => {
                    self.messages_input_buf.push(c);
                }
                _ => {}
            }
            return Some(format!("iMessage draft: {}", self.messages_input_buf));
        }

        if app.eq_ignore_ascii_case("Notes") {
            if let Some(idx) = self.notes_selected {
                if let Some((_, body)) = self.notes_items.get_mut(idx) {
                    match ch {
                        '\x08' => {
                            body.pop();
                        }
                        '\r' | '\n' => {
                            body.push('\n');
                        }
                        c if !c.is_control() && body.len() < 240 => {
                            body.push(c);
                        }
                        _ => {}
                    }
                    return Some("Updated Note".to_string());
                }
            }
        }
        None
    }

    pub(crate) fn start_phone_call(&mut self, custom_target: Option<String>) {
        let target = custom_target.unwrap_or_else(|| {
            if self.phone_dial_buf.is_empty() {
                "198 (Viettel Care)".to_string()
            } else {
                self.phone_dial_buf.clone()
            }
        });
        self.phone_call_target = target.clone();
        self.phone_in_call = true;
        self.phone_call_start_tick = self.frame_tick;
        self.phone_muted = false;
    }

    pub(crate) fn end_phone_call(&mut self) {
        if self.phone_in_call {
            self.phone_in_call = false;
            let target = if self.phone_call_target.is_empty() {
                "Unknown".to_string()
            } else {
                self.phone_call_target.clone()
            };
            self.phone_recents
                .insert(0, (target, "Outgoing • Just now".to_string()));
            if self.phone_recents.len() > 8 {
                self.phone_recents.pop();
            }
        }
    }

    pub(crate) fn commit_safari_url(&mut self) {
        self.safari_editing_url = false;
        let raw = self.safari_input_buf.trim().to_string();
        if raw.is_empty() {
            return;
        }
        if !self.safari_url.is_empty() {
            self.safari_history.push(self.safari_url.clone());
        }
        let lower = raw.to_lowercase();
        self.safari_url = if lower.contains("apple") {
            "https://www.apple.com/iphone-x".to_string()
        } else if lower.contains("google") {
            "https://www.google.com".to_string()
        } else if lower.contains("vnexpress") {
            "https://vnexpress.net".to_string()
        } else if lower.contains("wiki") {
            "https://en.wikipedia.org/wiki/IOS_16".to_string()
        } else if lower.contains("github") {
            "https://github.com/khanh/ios-sweet".to_string()
        } else if lower.contains("youtube") {
            "https://m.youtube.com".to_string()
        } else if lower.starts_with("http://") || lower.starts_with("https://") || lower.contains('.') {
            raw
        } else {
            format!("https://www.google.com/search?q={}", raw)
        };
        self.scroll_y = 0;
    }

    pub(crate) fn send_active_imessage(&mut self) -> Option<String> {
        let idx = self.messages_active_chat?;
        let msg = self.messages_input_buf.trim().to_string();
        if msg.is_empty() {
            return None;
        }
        self.messages_input_buf.clear();
        if let Some((contact, time_str, msgs)) = self.messages_threads.get_mut(idx) {
            *time_str = "9:41 AM".to_string();
            msgs.push((true, msg.clone()));
            let reply = match contact.as_str() {
                "Apple" => "Apple iMessage: Your iPhone X (iOS 16.7.16) is verified and active.",
                "Khanh (iPhone X)" => "Ok em! Dien thoai bam muot lam, ngon lanh!",
                "Viettel" => "Viettel: Quy khach con 9.8 GB data 5G toc do cao.",
                _ => "Team ios-sweet: Received your message loud & clear!",
            };
            msgs.push((false, reply.to_string()));
            return Some(format!("Sent iMessage to {}: '{}'", contact, msg));
        }
        None
    }

    pub(crate) fn calc_press(&mut self, btn: &str) {
        match btn {
            "AC" | "C" => {
                self.calc_display = "0".to_string();
                self.calc_acc = 0.0;
                self.calc_pending_op = None;
                self.calc_replace_next = true;
            }
            "+/-" => {
                if let Ok(v) = self.calc_display.parse::<f64>() {
                    self.calc_display = format_calc_number(-v);
                }
            }
            "%" => {
                if let Ok(v) = self.calc_display.parse::<f64>() {
                    self.calc_display = format_calc_number(v / 100.0);
                    self.calc_replace_next = true;
                }
            }
            "." => {
                if self.calc_replace_next {
                    self.calc_display = "0.".to_string();
                    self.calc_replace_next = false;
                } else if !self.calc_display.contains('.') && self.calc_display.len() < 10 {
                    self.calc_display.push('.');
                }
            }
            "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
                if self.calc_replace_next || self.calc_display == "0" {
                    self.calc_display = btn.to_string();
                    self.calc_replace_next = false;
                } else if self.calc_display.len() < 9 {
                    self.calc_display.push_str(btn);
                }
            }
            "+" | "-" | "×" | "x" | "÷" | "/" => {
                let op_ch = match btn {
                    "×" | "x" => '*',
                    "÷" | "/" => '/',
                    "+" => '+',
                    _ => '-',
                };
                let cur = self.calc_display.parse::<f64>().unwrap_or(0.0);
                if let Some(prev_op) = self.calc_pending_op {
                    if !self.calc_replace_next {
                        self.calc_acc = eval_calc_op(self.calc_acc, cur, prev_op);
                        self.calc_display = format_calc_number(self.calc_acc);
                    }
                } else {
                    self.calc_acc = cur;
                }
                self.calc_pending_op = Some(op_ch);
                self.calc_replace_next = true;
            }
            "=" => {
                if let Some(op) = self.calc_pending_op.take() {
                    let cur = self.calc_display.parse::<f64>().unwrap_or(0.0);
                    let res = eval_calc_op(self.calc_acc, cur, op);
                    self.calc_acc = res;
                    self.calc_display = format_calc_number(res);
                    self.calc_replace_next = true;
                }
            }
            _ => {}
        }
    }
}

fn eval_calc_op(a: f64, b: f64, op: char) -> f64 {
    match op {
        '+' => a + b,
        '-' => a - b,
        '*' => a * b,
        '/' => {
            if b.abs() < 1e-12 {
                0.0
            } else {
                a / b
            }
        }
        _ => b,
    }
}

fn format_calc_number(v: f64) -> String {
    if !v.is_finite() {
        return "Error".to_string();
    }
    if (v - v.round()).abs() < 1e-9 && v.abs() < 1e12 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{:.6}", v);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s.len() > 10 {
            s[..10].to_string()
        } else {
            s.to_string()
        }
    }
}
impl IosSimState {
    /// Handles a tap at `(vx, vy)` in `360x800` viewport coordinates across Lock Screen,
    /// Home Screen, Control Center, Spotlight, and all 24+ interactive iOS 16 apps.
    pub fn handle_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        self.ensure_initialized();

        // Top-right ear swipe/tap (x >= 260, y <= 28) opens Control Center from anywhere
        if vx >= 260 && vy <= 28 && !self.locked {
            if self.active_app.as_deref() == Some("Control Center") {
                self.active_app = None;
                return Some("Closed Control Center".to_string());
            } else {
                self.active_app = Some("Control Center".to_string());
                return Some("Opened Control Center".to_string());
            }
        }

        // Bottom Home Indicator bar (y >= 776) acts as Home / Back gesture
        if vy >= 776 {
            return self.handle_back_gesture();
        }

        // 1. Lock Screen interactions
        if self.locked {
            // Bottom-left Flashlight button (x: 24..86, y: 706..768)
            if (24..=86).contains(&vx) && (706..=768).contains(&vy) {
                self.flashlight_on = !self.flashlight_on;
                return Some(format!(
                    "Lock Screen Flashlight: {}",
                    if self.flashlight_on { "ON" } else { "OFF" }
                ));
            }
            // Bottom-right Camera button (x: 274..336, y: 706..768)
            if (274..=336).contains(&vx) && (706..=768).contains(&vy) {
                self.locked = false;
                self.active_app = Some("Camera".to_string());
                return Some("Lock Screen -> Camera".to_string());
            }
            self.locked = false;
            return Some("Unlocked iPhone X -> Home Screen".to_string());
        }

        // 2. Spotlight Search overlay interactions
        if self.spotlight_open {
            if let Some(key) = hit_test_ios_keyboard(vx, vy) {
                return self.handle_key(key);
            }
            // Cancel button at top-right (x >= 285, y: 42..82)
            if vx >= 285 && (42..=82).contains(&vy) {
                self.spotlight_open = false;
                return Some("Closed Spotlight Search".to_string());
            }
            // Tapping a result row (y: 105..360)
            if (105..=360).contains(&vy) {
                let row_idx = ((vy - 105) / 48) as usize;
                let matches = self.matching_spotlight_apps();
                if let Some(app) = matches.get(row_idx) {
                    let target = app.clone();
                    self.spotlight_open = false;
                    self.active_app = Some(target.clone());
                    self.scroll_y = 0;
                    return Some(format!("Launched {} from Spotlight", target));
                }
            }
            self.spotlight_open = false;
            return Some("Dismissed Spotlight".to_string());
        }

        // 3. Home Screen interactions (when no app is active)
        if self.active_app.is_none() {
            // Search Pill (x: 130..230, y: 650..688)
            if (130..=230).contains(&vx) && (650..=688).contains(&vy) {
                self.spotlight_open = true;
                self.spotlight_query.clear();
                return Some("Opened Spotlight Search".to_string());
            }
            // Bottom Dock (y: 694..774, x: 12..348)
            if (694..=774).contains(&vy) && (12..=348).contains(&vx) {
                let dock_idx = (((vx - 12) * 4) / 336).clamp(0, 3) as usize;
                let app = DOCK_APPS[dock_idx];
                self.active_app = Some(app.to_string());
                self.scroll_y = 0;
                return Some(format!("Opened {} from Dock", app));
            }
            // 4x6 Home Screen Icon Grid
            let has_row6 = !self.installed_apps.is_empty();
            let grid_top = if has_row6 { 48 } else { 54 };
            let row_h = if has_row6 { 78 } else { 84 };
            if vy >= grid_top && vy < 648 && (14..=346).contains(&vx) {
                let col = (((vx - 18) * 4) / 324).clamp(0, 3) as usize;
                let row = ((vy - grid_top) / row_h).max(0) as usize;
                let idx = row * 4 + col;
                let mut all_icons: Vec<String> =
                    HOME_GRID_APPS.iter().map(|&s| s.to_string()).collect();
                for a in &self.installed_apps {
                    let nm = a.display_name.trim();
                    if !nm.is_empty() && !nm.eq_ignore_ascii_case("Sweet 3D Bench") {
                        all_icons.push(nm.to_string());
                    }
                }
                if let Some(chosen) = all_icons.get(idx) {
                    self.active_app = Some(chosen.clone());
                    self.scroll_y = 0;
                    self.settings_page.clear();
                    return Some(format!("Opened {}", chosen));
                }
            }
            return None;
        }

        // 4. Active App interactions
        let app = self.active_app.clone().unwrap_or_default();
        match app.as_str() {
            "Settings" => self.handle_settings_tap(vx, vy),
            "Phone" => self.handle_phone_tap(vx, vy),
            "Calculator" => self.handle_calculator_tap(vx, vy),
            "Safari" => self.handle_safari_tap(vx, vy),
            "Messages" => self.handle_messages_tap(vx, vy),
            "Camera" => self.handle_camera_tap(vx, vy),
            "Photos" => self.handle_photos_tap(vx, vy),
            "Clock" => self.handle_clock_tap(vx, vy),
            "Notes" => self.handle_notes_tap(vx, vy),
            "Reminders" => self.handle_reminders_tap(vx, vy),
            "Music" => self.handle_music_tap(vx, vy),
            "Weather" => self.handle_weather_tap(vx, vy),
            "Files" => self.handle_files_tap(vx, vy),
            "App Store" => self.handle_app_store_tap(vx, vy),
            "Control Center" => self.handle_control_center_tap(vx, vy),
            _ => self.handle_other_app_tap(&app, vx, vy),
        }
    }

    pub(crate) fn matching_spotlight_apps(&self) -> Vec<String> {
        let q = self.spotlight_query.trim().to_lowercase();
        let mut out = Vec::new();
        for &app in HOME_GRID_APPS.iter().chain(DOCK_APPS.iter()) {
            if q.is_empty() || app.to_lowercase().contains(&q) {
                out.push(app.to_string());
                if out.len() >= 5 {
                    break;
                }
            }
        }
        out
    }

    fn handle_settings_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        // Subpage Back button at top-left (x: 8..125, y: 34..72)
        if !self.settings_page.is_empty()
            && self.settings_page != "Main"
            && vx <= 125
            && (34..=72).contains(&vy)
        {
            return self.handle_back_gesture();
        }

        match self.settings_page.as_str() {
            "" | "Main" => {
                let eff_y = vy + self.scroll_y;
                // Apple ID Profile Card (eff_y: 104..162)
                if (104..=162).contains(&eff_y) {
                    self.settings_page = "AppleID".to_string();
                    self.scroll_y = 0;
                    return Some("Settings -> Apple ID (iCloud+)".to_string());
                }
                // Group 1 (eff_y: 172..316): Airplane Mode, Wi-Fi, Bluetooth, Cellular
                if (172..=316).contains(&eff_y) {
                    let row = (eff_y - 172) / 36;
                    return match row {
                        0 => {
                            self.airplane_mode = !self.airplane_mode;
                            if self.airplane_mode {
                                self.cellular_enabled = false;
                            } else {
                                self.cellular_enabled = true;
                            }
                            Some(format!(
                                "Airplane Mode: {}",
                                if self.airplane_mode { "ON" } else { "OFF" }
                            ))
                        }
                        1 => {
                            self.settings_page = "WiFi".to_string();
                            self.scroll_y = 0;
                            Some("Settings -> Wi-Fi".to_string())
                        }
                        2 => {
                            self.settings_page = "Bluetooth".to_string();
                            self.scroll_y = 0;
                            Some("Settings -> Bluetooth".to_string())
                        }
                        _ => {
                            self.settings_page = "Cellular".to_string();
                            self.scroll_y = 0;
                            Some("Settings -> Cellular".to_string())
                        }
                    };
                }
                // Group 2 (eff_y: 328..472): Notifications, Sounds & Haptics, Focus, Screen Time
                if (328..=472).contains(&eff_y) {
                    let row = (eff_y - 328) / 36;
                    self.scroll_y = 0;
                    return match row {
                        0 => {
                            self.settings_page = "Notifications".to_string();
                            Some("Settings -> Notifications".to_string())
                        }
                        1 => {
                            self.settings_page = "Sounds".to_string();
                            Some("Settings -> Sounds & Haptics".to_string())
                        }
                        2 => {
                            self.settings_page = "Focus".to_string();
                            Some("Settings -> Focus".to_string())
                        }
                        _ => {
                            self.settings_page = "ScreenTime".to_string();
                            Some("Settings -> Screen Time".to_string())
                        }
                    };
                }
                // Group 3 (eff_y: 484..664): General, Control Center, Display & Brightness, Wallpaper, Battery
                if (484..=664).contains(&eff_y) {
                    let row = (eff_y - 484) / 36;
                    self.scroll_y = 0;
                    return match row {
                        0 => {
                            self.settings_page = "General".to_string();
                            Some("Settings -> General".to_string())
                        }
                        1 => {
                            self.settings_page = "ControlCenter".to_string();
                            Some("Settings -> Control Center".to_string())
                        }
                        2 => {
                            self.settings_page = "Display".to_string();
                            Some("Settings -> Display & Brightness".to_string())
                        }
                        3 => {
                            self.settings_page = "Wallpaper".to_string();
                            Some("Settings -> Wallpaper".to_string())
                        }
                        _ => {
                            self.settings_page = "Battery".to_string();
                            Some("Settings -> Battery".to_string())
                        }
                    };
                }
                None
            }
            "WiFi" => {
                if (86..=126).contains(&vy) {
                    self.wifi_enabled = !self.wifi_enabled;
                    return Some(format!(
                        "Wi-Fi: {}",
                        if self.wifi_enabled { "ON" } else { "OFF" }
                    ));
                }
                if self.wifi_enabled && (204..=364).contains(&vy) {
                    let nets = [
                        "SweetNet-5G",
                        "Viettel-5G-Ultra",
                        "VNPT-Fiber-Home",
                        "FPT-Telecom-5G",
                    ];
                    let idx = (((vy - 204) / 40) as usize).min(nets.len() - 1);
                    self.wifi_network = nets[idx].to_string();
                    return Some(format!("Connected to Wi-Fi: {}", self.wifi_network));
                }
                None
            }
            "Bluetooth" => {
                if (86..=126).contains(&vy) {
                    self.bluetooth_enabled = !self.bluetooth_enabled;
                    return Some(format!(
                        "Bluetooth: {}",
                        if self.bluetooth_enabled { "ON" } else { "OFF" }
                    ));
                }
                None
            }
            "Cellular" => {
                if (86..=126).contains(&vy) {
                    self.cellular_enabled = !self.cellular_enabled;
                    return Some(format!(
                        "Cellular Data: {}",
                        if self.cellular_enabled { "ON" } else { "OFF" }
                    ));
                }
                None
            }
            "General" => {
                if (86..=374).contains(&vy) {
                    let row = (vy - 86) / 36;
                    return match row {
                        0 => {
                            self.settings_page = "About".to_string();
                            Some("Settings -> General -> About".to_string())
                        }
                        1 => {
                            self.settings_page = "SoftwareUpdate".to_string();
                            Some("Settings -> General -> Software Update".to_string())
                        }
                        3 => {
                            self.settings_page = "Storage".to_string();
                            Some("Settings -> General -> iPhone Storage".to_string())
                        }
                        _ => Some("Settings -> General option selected".to_string()),
                    };
                }
                None
            }
            "Display" => {
                // Light vs Dark mode cards (y: 100..215)
                if (100..=215).contains(&vy) {
                    self.dark_mode = vx >= 180;
                    return Some(format!(
                        "Appearance: {}",
                        if self.dark_mode { "Dark Mode" } else { "Light Mode" }
                    ));
                }
                // Brightness horizontal slider (y: 235..285)
                if (235..=285).contains(&vy) && (32..=328).contains(&vx) {
                    self.brightness = (((vx - 44) * 100 / 272).clamp(15, 100)) as u8;
                    return Some(format!("Brightness: {}%", self.brightness));
                }
                // True Tone toggle (y: 292..330)
                if (292..=330).contains(&vy) {
                    self.true_tone = !self.true_tone;
                    return Some(format!(
                        "True Tone: {}",
                        if self.true_tone { "ON" } else { "OFF" }
                    ));
                }
                // Night Shift toggle (y: 331..370)
                if (331..=370).contains(&vy) {
                    self.night_shift = !self.night_shift;
                    return Some(format!(
                        "Night Shift: {}",
                        if self.night_shift { "ON" } else { "OFF" }
                    ));
                }
                None
            }
            "Wallpaper" => {
                if (110..=360).contains(&vy) {
                    self.dark_mode = vx >= 180;
                    return Some(format!(
                        "Switched Wallpaper to {}",
                        if self.dark_mode {
                            "iOS 16 Dark (SydneyDark)"
                        } else {
                            "iOS 16 Light (SydneyLight)"
                        }
                    ));
                }
                None
            }
            "Battery" => {
                if (122..=162).contains(&vy) {
                    self.low_power_mode = !self.low_power_mode;
                    return Some(format!(
                        "Low Power Mode: {}",
                        if self.low_power_mode { "ON" } else { "OFF" }
                    ));
                }
                None
            }
            "Sounds" => {
                if (130..=180).contains(&vy) && (32..=328).contains(&vx) {
                    self.volume = (((vx - 44) * 100 / 272).clamp(0, 100)) as u8;
                    return Some(format!("Ringer & Alerts Volume: {}%", self.volume));
                }
                None
            }
            "Focus" => {
                if (86..=130).contains(&vy) {
                    self.focus_dnd = !self.focus_dnd;
                    return Some(format!(
                        "Do Not Disturb: {}",
                        if self.focus_dnd { "ON" } else { "OFF" }
                    ));
                }
                None
            }
            _ => None,
        }
    }

    fn handle_phone_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        if self.phone_in_call {
            // 6 call action buttons (2 rows of 3: y: 240..420)
            if (240..=320).contains(&vy) {
                if vx < 130 {
                    self.phone_muted = !self.phone_muted;
                    return Some(format!(
                        "Call Mute: {}",
                        if self.phone_muted { "ON" } else { "OFF" }
                    ));
                } else if vx > 230 {
                    self.phone_speaker = !self.phone_speaker;
                    return Some(format!(
                        "Speakerphone: {}",
                        if self.phone_speaker { "ON" } else { "OFF" }
                    ));
                }
            }
            // Red End Call circle (y: 470..555)
            if (465..=560).contains(&vy) && (135..=225).contains(&vx) {
                self.end_phone_call();
                return Some("Ended Phone Call".to_string());
            }
            return None;
        }

        // Bottom 5 tabs (y: 716..775): Favorites, Recents, Contacts, Keypad, Voicemail
        if (716..=775).contains(&vy) {
            self.phone_tab = ((vx * 5) / 360).clamp(0, 4) as u8;
            return Some(format!(
                "Phone Tab -> {}",
                ["Favorites", "Recents", "Contacts", "Keypad", "Voicemail"]
                    [self.phone_tab as usize]
            ));
        }

        if self.phone_tab == 3 {
            // Keypad 4x3 grid: start_x = 92, col_pitch = 88, grid_top = 165, row_pitch = 80
            let keys = [
                '1', '2', '3', '4', '5', '6', '7', '8', '9', '*', '0', '#',
            ];
            for (i, &ch) in keys.iter().enumerate() {
                let col = (i as i32) % 3;
                let row = (i as i32) / 3;
                let cx = 92 + col * 88;
                let cy = 165 + row * 80;
                let dx = vx - cx;
                let dy = vy - cy;
                if dx * dx + dy * dy <= 38 * 38 {
                    if self.phone_dial_buf.len() < 16 {
                        self.phone_dial_buf.push(ch);
                    }
                    return Some(format!("Dialed {}", self.phone_dial_buf));
                }
            }
            // Green Call button at (180, 492), r = 36
            let dx_call = vx - 180;
            let dy_call = vy - 492;
            if dx_call * dx_call + dy_call * dy_call <= 40 * 40 {
                self.start_phone_call(None);
                return Some(format!("Calling {}...", self.phone_call_target));
            }
            // Backspace button at right of Call button (x: 245..325, y: 465..520)
            if (245..=325).contains(&vx) && (465..=520).contains(&vy) {
                self.phone_dial_buf.pop();
                return Some(format!("Dial buffer: {}", self.phone_dial_buf));
            }
        } else if (95..=460).contains(&vy) {
            // Tapping a contact/recent row initiates a call
            let idx = ((vy - 95) / 56) as usize;
            let contacts = [
                "Khanh (iPhone X)",
                "Apple Support",
                "Viettel 198",
                "Team ios-sweet",
                "EmergencySOS",
            ];
            let target = self
                .phone_recents
                .get(idx)
                .map(|(n, _)| n.clone())
                .unwrap_or_else(|| contacts[idx.min(contacts.len() - 1)].to_string());
            self.start_phone_call(Some(target.clone()));
            return Some(format!("Calling {}...", target));
        }
        None
    }

    fn handle_calculator_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        let rows: [[&str; 4]; 5] = [
            ["AC", "+/-", "%", "÷"],
            ["7", "8", "9", "×"],
            ["4", "5", "6", "-"],
            ["1", "2", "3", "+"],
            ["0", "0", ".", "="],
        ];
        if !(245..=685).contains(&vy) {
            return None;
        }
        let r_idx = ((vy - 245) / 86).clamp(0, 4) as usize;
        let c_idx = ((vx - 12) / 85).clamp(0, 3) as usize;
        let btn = rows[r_idx][c_idx];
        self.calc_press(btn);
        Some(format!("Calculator [{}] -> {}", btn, self.calc_display))
    }

    fn handle_safari_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        if self.safari_editing_url {
            if let Some(ch) = hit_test_ios_keyboard(vx, vy) {
                return self.handle_key(ch);
            }
            if vy < 440 {
                self.safari_editing_url = false;
                return Some("Dismissed Safari Keyboard".to_string());
            }
        }

        // Bottom Address Bar (y: 685..732)
        if (685..=732).contains(&vy) {
            // Reload button on right of address bar (x >= 305)
            if vx >= 305 && !self.safari_url.is_empty() {
                return Some(format!("Reloaded {}", self.safari_url));
            }
            self.safari_editing_url = true;
            self.safari_input_buf.clear();
            return Some("Editing Safari URL / Search".to_string());
        }

        // Bottom Safari Navigation Toolbar (y: 733..775)
        if (733..=775).contains(&vy) {
            if vx < 85 {
                // '<' Back to Start Page / previous page
                self.safari_url = self.safari_history.pop().unwrap_or_default();
                self.scroll_y = 0;
                return Some("Safari -> Back".to_string());
            } else if vx > 275 {
                // Tabs / Start Page reset
                self.safari_url.clear();
                self.scroll_y = 0;
                return Some("Safari -> Favorites Start Page".to_string());
            }
        }

        // Start Page Favorites Grid (when safari_url is empty)
        if self.safari_url.is_empty() && (86..=260).contains(&vy) && (16..=344).contains(&vx) {
            let col = (((vx - 20) * 4) / 320).clamp(0, 3) as usize;
            let row = ((vy - 86) / 84).clamp(0, 1) as usize;
            let idx = row * 4 + col;
            let urls = [
                "https://www.apple.com/iphone-x",
                "https://www.google.com",
                "https://vnexpress.net",
                "https://en.wikipedia.org/wiki/IOS_16",
                "https://github.com/khanh/ios-sweet",
                "https://m.youtube.com",
            ];
            if let Some(&u) = urls.get(idx) {
                self.safari_url = u.to_string();
                self.scroll_y = 0;
                return Some(format!("Safari opened {}", u));
            }
        }
        None
    }

    fn handle_messages_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        if self.messages_active_chat.is_some() {
            // '< Messages' back button at top-left (x <= 115, y: 34..76)
            if vx <= 115 && (34..=76).contains(&vy) {
                self.messages_active_chat = None;
                return Some("Messages -> Conversations".to_string());
            }
            // Blue Send button (x >= 305, y: 455..505)
            if vx >= 305 && (455..=505).contains(&vy) {
                return self.send_active_imessage();
            }
            // On-screen keyboard keys
            if let Some(ch) = hit_test_ios_keyboard(vx, vy) {
                return self.handle_key(ch);
            }
            return None;
        }

        // Conversation list rows (y: 112..380)
        if (112..=380).contains(&vy) {
            let idx = ((vy - 112) / 64) as usize;
            if idx < self.messages_threads.len() {
                self.messages_active_chat = Some(idx);
                let name = self.messages_threads[idx].0.clone();
                return Some(format!("Opened iMessage chat with {}", name));
            }
        }
        None
    }

    fn handle_camera_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        // Mode selector bar (y: 635..675)
        if (635..=675).contains(&vy) {
            self.camera_mode = ((vx * 4) / 360).clamp(0, 3) as u8;
            let mode_name = ["VIDEO", "PHOTO", "PORTRAIT", "PANO"][self.camera_mode as usize];
            return Some(format!("Camera Mode -> {}", mode_name));
        }
        // Bottom-left Photo Thumbnail -> opens Photos app (x: 20..88, y: 688..756)
        if (20..=88).contains(&vx) && (688..=756).contains(&vy) {
            self.active_app = Some("Photos".to_string());
            self.photos_selected = Some(self.photos_count.saturating_sub(1));
            return Some("Camera -> Opened Photos Preview".to_string());
        }
        // Center White Shutter Button (x: 140..220, y: 684..764)
        if (140..=220).contains(&vx) && (684..=764).contains(&vy) {
            self.camera_flash_tick = self.frame_tick;
            self.photos_count = (self.photos_count + 1).min(18);
            return Some(format!(
                "Captured Photo #{} in Camera!",
                self.photos_count
            ));
        }
        // Bottom-right Flip Camera Button (x: 272..340, y: 688..756)
        if (272..=340).contains(&vx) && (688..=756).contains(&vy) {
            self.camera_front = !self.camera_front;
            return Some(format!(
                "Switched to {} Camera",
                if self.camera_front {
                    "Front TrueDepth"
                } else {
                    "Rear Dual 12MP"
                }
            ));
        }
        None
    }

    fn handle_photos_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        if let Some(sel) = self.photos_selected {
            // Back button '< Library' at top-left (x <= 120, y: 34..74)
            if vx <= 120 && (34..=74).contains(&vy) {
                self.photos_selected = None;
                return Some("Photos -> Library Grid".to_string());
            }
            // Bottom toolbar: Favorite heart (center) & Trash (right)
            if (715..=772).contains(&vy) {
                if (145..=215).contains(&vx) {
                    self.photos_favorites ^= 1 << (sel % 16);
                    return Some(format!("Toggled Favorite on Photo #{}", sel + 1));
                } else if vx >= 280 && self.photos_count > 1 {
                    self.photos_count -= 1;
                    self.photos_selected = None;
                    return Some("Deleted Photo".to_string());
                }
            }
            return None;
        }

        // Bottom 4 tabs (y: 718..775): Library, For You, Albums, Search
        if (718..=775).contains(&vy) {
            self.photos_tab = ((vx * 4) / 360).clamp(0, 3) as u8;
            return Some(format!(
                "Photos Tab -> {}",
                ["Library", "For You", "Albums", "Search"][self.photos_tab as usize]
            ));
        }

        // 3-column Photo Grid (y: 96..690)
        if (96..=690).contains(&vy) && (12..=348).contains(&vx) {
            let col = (((vx - 12) * 3) / 336).clamp(0, 2) as usize;
            let row = ((vy - 96) / 112) as usize;
            let idx = row * 3 + col;
            if idx < self.photos_count {
                self.photos_selected = Some(idx);
                return Some(format!("Viewing Photo #{}", idx + 1));
            }
        }
        None
    }

    fn handle_clock_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        // Bottom 4 tabs (y: 718..775): World Clock, Alarm, Stopwatch, Timer
        if (718..=775).contains(&vy) {
            self.clock_tab = ((vx * 4) / 360).clamp(0, 3) as u8;
            return Some(format!(
                "Clock Tab -> {}",
                ["World Clock", "Alarm", "Stopwatch", "Timer"][self.clock_tab as usize]
            ));
        }
        match self.clock_tab {
            1 => {
                // Alarm toggles (y: 96..350)
                if (96..=350).contains(&vy) {
                    let idx = (((vy - 96) / 62) as usize).min(3);
                    self.alarms_enabled ^= 1 << idx;
                    return Some(format!("Toggled Alarm #{}", idx + 1));
                }
            }
            2 => {
                // Stopwatch Lap/Reset (left) & Start/Stop (right) at y: 310..390
                if (310..=390).contains(&vy) {
                    if vx < 160 {
                        if self.stopwatch_running {
                            self.stopwatch_laps.insert(0, self.stopwatch_elapsed_ticks);
                            self.stopwatch_laps.truncate(5);
                            return Some("Recorded Stopwatch Lap".to_string());
                        } else {
                            self.stopwatch_elapsed_ticks = 0;
                            self.stopwatch_laps.clear();
                            return Some("Reset Stopwatch".to_string());
                        }
                    } else if vx > 200 {
                        self.stopwatch_running = !self.stopwatch_running;
                        return Some(format!(
                            "Stopwatch {}",
                            if self.stopwatch_running {
                                "Started"
                            } else {
                                "Stopped"
                            }
                        ));
                    }
                }
            }
            3 => {
                // Timer Cancel (left) & Start/Pause (right) at y: 410..490
                if (410..=490).contains(&vy) {
                    if vx < 160 {
                        self.timer_running = false;
                        self.timer_remaining_ticks = 15 * 60 * 60;
                        return Some("Reset Timer to 15:00".to_string());
                    } else if vx > 200 {
                        self.timer_running = !self.timer_running;
                        return Some(format!(
                            "Timer {}",
                            if self.timer_running {
                                "Running"
                            } else {
                                "Paused"
                            }
                        ));
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn handle_notes_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        if self.notes_selected.is_some() {
            if vx <= 115 && (34..=74).contains(&vy) {
                self.notes_selected = None;
                return Some("Notes -> All iCloud".to_string());
            }
            if let Some(ch) = hit_test_ios_keyboard(vx, vy) {
                return self.handle_key(ch);
            }
            return None;
        }
        // '+' New Note button at top-right or bottom-right
        if (vx >= 290 && (36..=78).contains(&vy)) || (vx >= 290 && (700..=765).contains(&vy)) {
            let n = self.notes_items.len() + 1;
            self.notes_items.push((
                format!("New Note #{}", n),
                "Typed on iPhone X (iOS 16.7.16):\n".to_string(),
            ));
            self.notes_selected = Some(self.notes_items.len() - 1);
            return Some(format!("Created New Note #{}", n));
        }
        if (106..=360).contains(&vy) {
            let idx = ((vy - 106) / 62) as usize;
            if idx < self.notes_items.len() {
                self.notes_selected = Some(idx);
                return Some(format!("Opened Note '{}'", self.notes_items[idx].0));
            }
        }
        None
    }

    fn handle_reminders_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        if vx >= 285 && (36..=78).contains(&vy) && self.reminders_items.len() < 8 {
            let n = self.reminders_items.len() + 1;
            self.reminders_items
                .push(format!("Custom Reminder Task #{}", n));
            return Some("Added New Reminder".to_string());
        }
        if (115..=480).contains(&vy) {
            let idx = ((vy - 115) / 50) as usize;
            if idx < self.reminders_items.len() {
                self.reminders_done ^= 1 << idx;
                return Some(format!(
                    "Toggled Reminder '{}'",
                    self.reminders_items[idx]
                ));
            }
        }
        None
    }

    fn handle_music_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        // Playback controls row (y: 525..595): Prev (x < 135), Play/Pause (135..225), Next (x > 225)
        if (525..=595).contains(&vy) {
            if vx < 135 {
                self.music_track_idx =
                    (self.music_track_idx + MUSIC_TRACKS.len() - 1) % MUSIC_TRACKS.len();
                self.music_progress_ticks = 0;
                return Some(format!(
                    "Apple Music: Previous -> {}",
                    MUSIC_TRACKS[self.music_track_idx].0
                ));
            } else if vx <= 225 {
                self.music_playing = !self.music_playing;
                return Some(format!(
                    "Apple Music: {}",
                    if self.music_playing {
                        "Playing"
                    } else {
                        "Paused"
                    }
                ));
            } else {
                self.music_track_idx = (self.music_track_idx + 1) % MUSIC_TRACKS.len();
                self.music_progress_ticks = 0;
                return Some(format!(
                    "Apple Music: Next -> {}",
                    MUSIC_TRACKS[self.music_track_idx].0
                ));
            }
        }
        // Volume bar (y: 625..665)
        if (625..=665).contains(&vy) && (40..=320).contains(&vx) {
            self.volume = (((vx - 48) * 100 / 264).clamp(0, 100)) as u8;
            return Some(format!("Music Volume: {}%", self.volume));
        }
        None
    }

    fn handle_weather_tap(&mut self, _vx: i32, _vy: i32) -> Option<String> {
        self.weather_city_idx = (self.weather_city_idx + 1) % WEATHER_CITIES.len();
        Some(format!(
            "Weather City -> {}",
            WEATHER_CITIES[self.weather_city_idx].0
        ))
    }

    fn handle_files_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        if !self.files_current_dir.is_empty() && vx <= 120 && (34..=74).contains(&vy) {
            self.files_current_dir.clear();
            return Some("Files -> Root (ios16-root-20H392.img)".to_string());
        }
        if (115..=520).contains(&vy) && self.files_current_dir.is_empty() {
            let dirs = [
                "Applications",
                "System",
                "Library",
                "sbin",
                "private",
                "Developer",
                "usr",
            ];
            let idx = ((vy - 115) / 52) as usize;
            if let Some(&d) = dirs.get(idx) {
                self.files_current_dir = d.to_string();
                return Some(format!("Files -> /{}", d));
            }
        }
        None
    }

    fn handle_app_store_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        if (718..=775).contains(&vy) {
            self.app_store_tab = ((vx * 4) / 360).clamp(0, 3) as u8;
            return Some(format!(
                "App Store Tab -> {}",
                ["Today", "Games", "Apps", "Search"][self.app_store_tab as usize]
            ));
        }
        // Tapping 'OPEN' button on featured app rows (y: 255..530, x >= 255)
        if (255..=530).contains(&vy) && vx >= 245 {
            let idx = ((vy - 255) / 68) as usize;
            let targets = ["Sweet 3D Bench", "Calculator", "Safari", "Music"];
            if let Some(&t) = targets.get(idx) {
                self.active_app = Some(t.to_string());
                return Some(format!("App Store -> Opened {}", t));
            }
        }
        None
    }

    fn handle_control_center_tap(&mut self, vx: i32, vy: i32) -> Option<String> {
        // Top-Left Connectivity Module (x: 24..173, y: 64..213)
        if (24..=173).contains(&vx) && (64..=213).contains(&vy) {
            let right_col = vx >= 98;
            let bot_row = vy >= 138;
            return match (bot_row, right_col) {
                (false, false) => {
                    self.airplane_mode = !self.airplane_mode;
                    Some(format!(
                        "Control Center: Airplane Mode {}",
                        if self.airplane_mode { "ON" } else { "OFF" }
                    ))
                }
                (false, true) => {
                    self.cellular_enabled = !self.cellular_enabled;
                    Some(format!(
                        "Control Center: Cellular Data {}",
                        if self.cellular_enabled { "ON" } else { "OFF" }
                    ))
                }
                (true, false) => {
                    self.wifi_enabled = !self.wifi_enabled;
                    Some(format!(
                        "Control Center: Wi-Fi {}",
                        if self.wifi_enabled { "ON" } else { "OFF" }
                    ))
                }
                (true, true) => {
                    self.bluetooth_enabled = !self.bluetooth_enabled;
                    Some(format!(
                        "Control Center: Bluetooth {}",
                        if self.bluetooth_enabled { "ON" } else { "OFF" }
                    ))
                }
            };
        }

        // Top-Right Music Module (x: 187..336, y: 64..213)
        if (187..=336).contains(&vx) && (64..=213).contains(&vy) {
            self.music_playing = !self.music_playing;
            return Some(format!(
                "Control Center Music: {}",
                if self.music_playing {
                    "Playing"
                } else {
                    "Paused"
                }
            ));
        }

        // Middle-Left Focus & Dark Mode tiles (x: 24..173, y: 227..376)
        if (24..=173).contains(&vx) && (227..=376).contains(&vy) {
            if vy < 301 {
                self.focus_dnd = !self.focus_dnd;
                return Some(format!(
                    "Control Center: Do Not Disturb {}",
                    if self.focus_dnd { "ON" } else { "OFF" }
                ));
            } else {
                self.dark_mode = !self.dark_mode;
                return Some(format!(
                    "Control Center: Dark Mode {}",
                    if self.dark_mode { "ON" } else { "OFF" }
                ));
            }
        }

        // Middle-Right Brightness & Volume Sliders (x: 187..336, y: 227..376)
        if (187..=336).contains(&vx) && (227..=376).contains(&vy) {
            let pct = ((376 - vy) * 100 / 149).clamp(10, 100) as u8;
            if vx < 261 {
                self.brightness = pct;
                return Some(format!("Control Center Brightness: {}%", self.brightness));
            } else {
                self.volume = pct;
                return Some(format!("Control Center Volume: {}%", self.volume));
            }
        }

        // Bottom 4 quick-action buttons (y: 390..465): Flashlight, Timer, Calculator, Camera
        if (390..=465).contains(&vy) && (24..=336).contains(&vx) {
            let q_idx = (((vx - 24) * 4) / 312).clamp(0, 3);
            return match q_idx {
                0 => {
                    self.flashlight_on = !self.flashlight_on;
                    Some(format!(
                        "Flashlight: {}",
                        if self.flashlight_on { "ON" } else { "OFF" }
                    ))
                }
                1 => {
                    self.active_app = Some("Clock".to_string());
                    self.clock_tab = 3;
                    Some("Control Center -> Timer".to_string())
                }
                2 => {
                    self.active_app = Some("Calculator".to_string());
                    Some("Control Center -> Calculator".to_string())
                }
                _ => {
                    self.active_app = Some("Camera".to_string());
                    Some("Control Center -> Camera".to_string())
                }
            };
        }

        // Tapping empty space below buttons dismisses Control Center
        if vy > 480 {
            self.active_app = None;
            return Some("Closed Control Center".to_string());
        }
        None
    }

    fn handle_other_app_tap(&mut self, app: &str, vx: i32, vy: i32) -> Option<String> {
        if app.eq_ignore_ascii_case("Calendar") && (120..=360).contains(&vy) {
            let col = (((vx - 16) * 7) / 328).clamp(0, 6) as u8;
            let row = (((vy - 120) / 38).clamp(0, 4)) as u8;
            self.calendar_selected_day = (row * 7 + col + 1).clamp(1, 31);
            return Some(format!(
                "Calendar: Selected October {}",
                self.calendar_selected_day
            ));
        }
        if app.eq_ignore_ascii_case("Home") && (125..=340).contains(&vy) {
            let col = if vx >= 180 { 1 } else { 0 };
            let row = if vy >= 230 { 1 } else { 0 };
            let bit = row * 2 + col;
            self.home_devices_on ^= 1 << bit;
            return Some("HomeKit: Toggled Smart Accessory".to_string());
        }
        if app.eq_ignore_ascii_case("Mail") {
            if self.mail_selected_idx.is_some() && vx <= 120 && (34..=74).contains(&vy) {
                self.mail_selected_idx = None;
                return Some("Mail -> Inbox".to_string());
            }
            if self.mail_selected_idx.is_none() && (110..=400).contains(&vy) {
                let idx = ((vy - 110) / 68) as usize;
                if idx < 4 {
                    self.mail_selected_idx = Some(idx);
                    return Some(format!("Mail -> Opened Message #{}", idx + 1));
                }
            }
        }
        if app.eq_ignore_ascii_case("FaceTime") && (100..=260).contains(&vy) {
            self.active_app = Some("Phone".to_string());
            self.start_phone_call(Some("FaceTime Audio • Khanh".to_string()));
            return Some("Started FaceTime Call".to_string());
        }
        None
    }
}

/// Hit-tests the on-screen iOS 16 QWERTY keyboard (`y: 515..770` on `360x800`).
pub(crate) fn hit_test_ios_keyboard(vx: i32, vy: i32) -> Option<char> {
    if !(518..=735).contains(&vy) {
        return None;
    }
    let row1 = [b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p'];
    let row2 = [b'a', b's', b'd', b'f', b'g', b'h', b'j', b'k', b'l'];
    let row3 = [b'z', b'x', b'c', b'v', b'b', b'n', b'm'];

    if (522..=568).contains(&vy) {
        let idx = (((vx - 6) * 10) / 348).clamp(0, 9) as usize;
        return Some(row1[idx] as char);
    }
    if (574..=620).contains(&vy) {
        let idx = (((vx - 22) * 9) / 316).clamp(0, 8) as usize;
        return Some(row2[idx] as char);
    }
    if (626..=672).contains(&vy) {
        if vx >= 302 {
            return Some('\x08'); // Backspace
        }
        if vx >= 54 {
            let idx = (((vx - 54) * 7) / 244).clamp(0, 6) as usize;
            return Some(row3[idx] as char);
        }
        return None;
    }
    if (678..=732).contains(&vy) {
        if vx >= 266 {
            return Some('\n'); // Return / Go / Send
        }
        if (92..=262).contains(&vx) {
            return Some(' '); // Space
        }
    }
    None
}

/// Renders the authentic iOS 16 On-Screen QWERTY Keyboard at `y = 514..770`.
pub(crate) fn draw_ios_keyboard(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    dark_mode: bool,
    return_label: &str,
) {
    let w = fb_w as i32;
    let kb_bg = if dark_mode {
        rgb(28, 28, 30)
    } else {
        rgb(209, 212, 219)
    };
    let key_bg = if dark_mode {
        rgb(72, 72, 74)
    } else {
        rgb(255, 255, 255)
    };
    let sp_bg = if dark_mode {
        rgb(48, 48, 52)
    } else {
        rgb(172, 176, 185)
    };
    let txt_col = if dark_mode {
        rgb(255, 255, 255)
    } else {
        rgb(18, 18, 22)
    };

    fill_rect(fb, fb_w, fb_h, 0, 514, w, 262, kb_bg);

    let row1 = ["Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P"];
    for (i, &k) in row1.iter().enumerate() {
        let kx = 6 + (i as i32) * 35;
        fill_rounded_rect(fb, fb_w, fb_h, kx, 524, 31, 42, 5, key_bg);
        let kw = measure_text_px(k, 1);
        draw_text_semibold(fb, fb_w, fb_h, kx + 15 - kw / 2, 538, k, txt_col, 1, 2, false);
    }

    let row2 = ["A", "S", "D", "F", "G", "H", "J", "K", "L"];
    for (i, &k) in row2.iter().enumerate() {
        let kx = 22 + (i as i32) * 35;
        fill_rounded_rect(fb, fb_w, fb_h, kx, 576, 31, 42, 5, key_bg);
        let kw = measure_text_px(k, 1);
        draw_text_semibold(fb, fb_w, fb_h, kx + 15 - kw / 2, 590, k, txt_col, 1, 2, false);
    }

    // Shift key
    fill_rounded_rect(fb, fb_w, fb_h, 6, 628, 40, 42, 5, sp_bg);
    draw_text_semibold(fb, fb_w, fb_h, 20, 642, "^", txt_col, 1, 2, false);

    let row3 = ["Z", "X", "C", "V", "B", "N", "M"];
    for (i, &k) in row3.iter().enumerate() {
        let kx = 56 + (i as i32) * 35;
        fill_rounded_rect(fb, fb_w, fb_h, kx, 628, 31, 42, 5, key_bg);
        let kw = measure_text_px(k, 1);
        draw_text_semibold(fb, fb_w, fb_h, kx + 15 - kw / 2, 642, k, txt_col, 1, 2, false);
    }

    // Backspace key
    fill_rounded_rect(fb, fb_w, fb_h, 306, 628, 48, 42, 5, sp_bg);
    draw_text_semibold(fb, fb_w, fb_h, 320, 642, "<-", txt_col, 1, 2, false);

    // Bottom row: 123, Space, Return/Go
    fill_rounded_rect(fb, fb_w, fb_h, 6, 680, 78, 42, 5, sp_bg);
    draw_text_semibold(fb, fb_w, fb_h, 30, 694, "123", txt_col, 1, 2, false);

    fill_rounded_rect(fb, fb_w, fb_h, 92, 680, 170, 42, 5, key_bg);
    draw_text_scaled(fb, fb_w, fb_h, 158, 694, "space", txt_col, 1, 2);

    fill_rounded_rect(fb, fb_w, fb_h, 270, 680, 84, 42, 5, rgb(0, 122, 255));
    let rw = measure_text_px(return_label, 1);
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        312 - rw / 2,
        694,
        return_label,
        rgb(255, 255, 255),
        1,
        2,
        false,
    );
}

/// Renders the Spotlight Search overlay when the user taps the Home Screen `Search` pill.
pub(crate) fn render_spotlight_overlay(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    let h = fb_h as i32;
    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        0,
        34,
        w,
        h - 34,
        0,
        12,
        rgba(18, 22, 34, 185),
        0,
    );

    // Top search bar + Cancel button
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        16,
        46,
        w - 94,
        34,
        10,
        rgba(255, 255, 255, 42),
    );
    let q_txt = if screen_state.spotlight_query.is_empty() {
        "Spotlight Search"
    } else {
        screen_state.spotlight_query.as_str()
    };
    draw_text_scaled(fb, fb_w, fb_h, 30, 56, q_txt, rgb(255, 255, 255), 1, 2);
    draw_text_scaled(
        fb,
        fb_w,
        fb_h,
        w - 68,
        56,
        "Cancel",
        rgb(10, 132, 255),
        1,
        2,
    );

    let matches = screen_state.matching_spotlight_apps();
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        16,
        96,
        w - 32,
        (matches.len() as i32 * 48 + 16).max(56),
        14,
        rgba(38, 42, 56, 215),
    );
    for (i, app_name) in matches.iter().enumerate() {
        let ry = 104 + (i as i32) * 48;
        render_squircle_icon(fb, fb_w, fb_h, 28, ry + 4, 36, false, |canvas, cw, _ch| {
            draw_stock_icon_canvas(canvas, cw, app_name);
        });
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            76,
            ry + 15,
            app_name,
            rgb(255, 255, 255),
            1,
            2,
            false,
        );
        draw_text_right(
            fb,
            fb_w,
            fb_h,
            w - 28,
            ry + 15,
            "Open >",
            rgb(142, 142, 147),
            1,
            2,
        );
    }

    draw_ios_keyboard(fb, fb_w, fb_h, true, "Search");
}
/// Dispatches rendering for the currently active iOS 16 app.
pub(crate) fn render_active_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
    app_name: &str,
) {
    match app_name {
        s if s.eq_ignore_ascii_case("Settings") => {
            if screen_state.nav_anim > 0.005 && screen_state.nav_anim < 0.992 {
                let mut prev_state = screen_state.clone();
                prev_state.settings_page = screen_state.prev_settings_page.clone();
                prev_state.scroll_y = 0;
                let mut prev_buf = vec![0u32; fb_w * fb_h];
                let mut next_buf = vec![0u32; fb_w * fb_h];
                render_settings_app(&mut prev_buf, fb_w, fb_h, &prev_state);
                render_settings_app(&mut next_buf, fb_w, fb_h, screen_state);

                let t = screen_state.nav_anim.clamp(0.0, 1.0);
                let ease = 1.0 - (1.0 - t).powi(3);
                let w = fb_w as i32;
                if screen_state.nav_dir >= 0 {
                    // Push: prev shifts left -30%, next slides in from right (w -> 0)
                    let prev_shift = ((w as f32) * 0.30 * ease) as i32;
                    let next_x = ((w as f32) * (1.0 - ease)) as i32;
                    for y in 0..fb_h {
                        let row_off = y * fb_w;
                        for x in 0..fb_w {
                            let xi = x as i32;
                            if xi >= next_x {
                                let sx = (xi - next_x).clamp(0, w - 1) as usize;
                                fb[row_off + x] = next_buf[row_off + sx];
                            } else {
                                let sx = (xi + prev_shift).clamp(0, w - 1) as usize;
                                let px = prev_buf[row_off + sx];
                                // Darken underlying page slightly + drop shadow near next_x
                                let dist = (next_x - xi).max(0);
                                let dim = if dist < 12 { 170 + dist * 5 } else { 232 } as u32;
                                let r = (((px >> 16) & 0xFF) * dim) >> 8;
                                let g = (((px >> 8) & 0xFF) * dim) >> 8;
                                let b = ((px & 0xFF) * dim) >> 8;
                                fb[row_off + x] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
                            }
                        }
                    }
                } else {
                    // Pop: prev (deeper page) slides out to right (0 -> w) over next (parent page)
                    let next_shift = ((w as f32) * 0.30 * (1.0 - ease)) as i32;
                    let prev_x = ((w as f32) * ease) as i32;
                    for y in 0..fb_h {
                        let row_off = y * fb_w;
                        for x in 0..fb_w {
                            let xi = x as i32;
                            if xi >= prev_x {
                                let sx = (xi - prev_x).clamp(0, w - 1) as usize;
                                fb[row_off + x] = prev_buf[row_off + sx];
                            } else {
                                let sx = (xi + next_shift).clamp(0, w - 1) as usize;
                                fb[row_off + x] = next_buf[row_off + sx];
                            }
                        }
                    }
                }
            } else {
                render_settings_app(fb, fb_w, fb_h, screen_state);
            }
        }
        s if s.eq_ignore_ascii_case("Phone") => render_phone_app(fb, fb_w, fb_h, screen_state),
        s if s.eq_ignore_ascii_case("Calculator") => {
            render_calculator_app(fb, fb_w, fb_h, screen_state)
        }
        s if s.eq_ignore_ascii_case("Safari") => render_safari_app(fb, fb_w, fb_h, screen_state),
        s if s.eq_ignore_ascii_case("Messages") => render_messages_app(fb, fb_w, fb_h, screen_state),
        s if s.eq_ignore_ascii_case("Camera") => render_camera_app(fb, fb_w, fb_h, screen_state),
        s if s.eq_ignore_ascii_case("Photos") => render_photos_app(fb, fb_w, fb_h, screen_state),
        s if s.eq_ignore_ascii_case("Clock") => render_clock_app(fb, fb_w, fb_h, screen_state),
        s if s.eq_ignore_ascii_case("Notes") => render_notes_app(fb, fb_w, fb_h, screen_state),
        s if s.eq_ignore_ascii_case("Reminders") => {
            render_reminders_app(fb, fb_w, fb_h, screen_state)
        }
        s if s.eq_ignore_ascii_case("Music") => render_music_app(fb, fb_w, fb_h, screen_state),
        s if s.eq_ignore_ascii_case("Weather") => render_weather_app(fb, fb_w, fb_h, screen_state),
        s if s.eq_ignore_ascii_case("Files") => render_files_app(fb, fb_w, fb_h, screen_state),
        s if s.eq_ignore_ascii_case("App Store") => {
            render_app_store_app(fb, fb_w, fb_h, screen_state)
        }
        s if s.eq_ignore_ascii_case("Control Center") => {
            render_control_center_app(fb, fb_w, fb_h, screen_state)
        }
        _ => render_specialized_stock_app(fb, fb_w, fb_h, screen_state, app_name),
    }
}

/// Draws a crisp vector iOS 16 Settings icon inside a `24x24` rounded square badge at `(bx, by)`.
fn draw_settings_row_badge(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    bx: i32,
    by: i32,
    badge_col: u32,
    kind: &str,
) {
    fill_rounded_rect(fb, fb_w, fb_h, bx, by, 24, 24, 6, badge_col);
    let cx = bx + 12;
    let cy = by + 12;
    let w = rgb(255, 255, 255);

    match kind {
        "Airplane Mode" => {
            // Crisp white airplane silhouette
            draw_line(fb, fb_w, fb_h, cx - 7, cy, cx + 7, cy, 2, w);
            fill_triangle(fb, fb_w, fb_h, cx - 1, cy, cx - 4, cy - 7, cx + 3, cy, w);
            fill_triangle(fb, fb_w, fb_h, cx - 1, cy, cx - 4, cy + 7, cx + 3, cy, w);
            draw_line(fb, fb_w, fb_h, cx - 6, cy - 3, cx - 6, cy + 3, 2, w);
        }
        "Wi-Fi" => {
            // Wi-Fi concentric upper signal waves + bottom dot
            fill_circle(fb, fb_w, fb_h, cx, cy + 5, 2, w);
            draw_line(fb, fb_w, fb_h, cx - 4, cy + 1, cx, cy - 2, 2, w);
            draw_line(fb, fb_w, fb_h, cx, cy - 2, cx + 4, cy + 1, 2, w);
            draw_line(fb, fb_w, fb_h, cx - 7, cy - 2, cx, cy - 6, 2, w);
            draw_line(fb, fb_w, fb_h, cx, cy - 6, cx + 7, cy - 2, 2, w);
        }
        "Bluetooth" => {
            // Bluetooth B-rune
            draw_line(fb, fb_w, fb_h, cx, cy - 7, cx, cy + 7, 2, w);
            draw_line(fb, fb_w, fb_h, cx, cy - 7, cx + 4, cy - 3, 1, w);
            draw_line(fb, fb_w, fb_h, cx + 4, cy - 3, cx - 4, cy + 3, 1, w);
            draw_line(fb, fb_w, fb_h, cx - 4, cy - 3, cx + 4, cy + 3, 1, w);
            draw_line(fb, fb_w, fb_h, cx + 4, cy + 3, cx, cy + 7, 1, w);
        }
        "Cellular" => {
            // 4 ascending cellular signal bars
            for b_idx in 0..4i32 {
                let bh = 4 + b_idx * 3;
                fill_rect(
                    fb,
                    fb_w,
                    fb_h,
                    cx - 7 + b_idx * 4,
                    cy + 6 - bh,
                    2,
                    bh,
                    w,
                );
            }
        }
        "Notifications" => {
            // Bell / Notification badge
            fill_rounded_rect(fb, fb_w, fb_h, cx - 5, cy - 4, 10, 8, 4, w);
            fill_rect(fb, fb_w, fb_h, cx - 7, cy + 3, 14, 2, w);
            fill_circle(fb, fb_w, fb_h, cx, cy + 6, 2, w);
        }
        "Sounds & Haptics" => {
            // Speaker cone + sound waves
            fill_rect(fb, fb_w, fb_h, cx - 6, cy - 2, 3, 5, w);
            fill_triangle(fb, fb_w, fb_h, cx - 4, cy, cx + 1, cy - 5, cx + 1, cy + 5, w);
            draw_line(fb, fb_w, fb_h, cx + 4, cy - 3, cx + 5, cy, 1, w);
            draw_line(fb, fb_w, fb_h, cx + 5, cy, cx + 4, cy + 3, 1, w);
        }
        "Focus" => {
            // Crescent moon
            fill_circle(fb, fb_w, fb_h, cx, cy, 6, w);
            fill_circle(fb, fb_w, fb_h, cx + 3, cy - 2, 5, badge_col);
        }
        "Screen Time" => {
            // Hourglass silhouette
            fill_triangle(fb, fb_w, fb_h, cx - 5, cy - 6, cx + 5, cy - 6, cx, cy, w);
            fill_triangle(fb, fb_w, fb_h, cx, cy, cx - 5, cy + 6, cx + 5, cy + 6, w);
        }
        "General" => {
            // Mechanical gear wheel with teeth and center cutout
            draw_line(fb, fb_w, fb_h, cx - 7, cy, cx + 7, cy, 2, w);
            draw_line(fb, fb_w, fb_h, cx, cy - 7, cx, cy + 7, 2, w);
            draw_line(fb, fb_w, fb_h, cx - 5, cy - 5, cx + 5, cy + 5, 2, w);
            draw_line(fb, fb_w, fb_h, cx - 5, cy + 5, cx + 5, cy - 5, 2, w);
            fill_circle(fb, fb_w, fb_h, cx, cy, 5, w);
            fill_circle(fb, fb_w, fb_h, cx, cy, 2, badge_col);
        }
        "Control Center" => {
            // Two mini toggle switches
            fill_rounded_rect(fb, fb_w, fb_h, cx - 6, cy - 6, 12, 5, 2, w);
            fill_circle(fb, fb_w, fb_h, cx + 3, cy - 4, 2, badge_col);
            fill_rounded_rect(fb, fb_w, fb_h, cx - 6, cy + 1, 12, 5, 2, w);
            fill_circle(fb, fb_w, fb_h, cx - 3, cy + 3, 2, badge_col);
        }
        "Display & Brightness" => {
            // Sun with rays
            draw_line(fb, fb_w, fb_h, cx - 7, cy, cx + 7, cy, 1, w);
            draw_line(fb, fb_w, fb_h, cx, cy - 7, cx, cy + 7, 1, w);
            draw_line(fb, fb_w, fb_h, cx - 5, cy - 5, cx + 5, cy + 5, 1, w);
            draw_line(fb, fb_w, fb_h, cx - 5, cy + 5, cx + 5, cy - 5, 1, w);
            fill_circle(fb, fb_w, fb_h, cx, cy, 4, w);
        }
        "Wallpaper" => {
            // Flower / Kaleidoscope petals
            fill_circle(fb, fb_w, fb_h, cx - 3, cy - 2, 3, rgba(255, 255, 255, 210));
            fill_circle(fb, fb_w, fb_h, cx + 3, cy - 2, 3, rgba(255, 255, 255, 210));
            fill_circle(fb, fb_w, fb_h, cx, cy + 3, 3, w);
        }
        "Battery" => {
            // Horizontal battery icon with lightning bolt
            fill_rounded_rect(fb, fb_w, fb_h, cx - 7, cy - 4, 13, 8, 2, w);
            fill_rect(fb, fb_w, fb_h, cx + 6, cy - 2, 2, 4, w);
        }
        _ => {
            draw_ring(fb, fb_w, fb_h, cx, cy, 3, 5, w);
        }
    }
}

/// Draws an authentic iOS 16 UISwitch toggle at `(x, y)` (`38x22`).
fn draw_ios_switch(fb: &mut [u32], fb_w: usize, fb_h: usize, x: i32, y: i32, on: bool) {
    let track_col = if on {
        rgb(52, 199, 89)
    } else {
        rgb(220, 220, 228)
    };
    fill_rounded_rect(fb, fb_w, fb_h, x, y, 38, 22, 11, track_col);
    let knob_cx = if on { x + 27 } else { x + 11 };
    fill_circle(fb, fb_w, fb_h, knob_cx, y + 11, 9, rgb(255, 255, 255));
}

/// Draws a standard iOS 16 Navigation Bar header with a blue `< Back` button on the left.
fn draw_subpage_header(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    back_label: &str,
    title: &str,
    dark_mode: bool,
) {
    let fg = if dark_mode {
        rgb(255, 255, 255)
    } else {
        rgb(18, 18, 22)
    };
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        14,
        46,
        &format!("< {}", back_label),
        rgb(0, 122, 255),
        1,
        2,
        false,
    );
    draw_text_centered(fb, fb_w, fb_h, 46, title, fg, 1, 2, false);
}

/// Renders the interactive iOS 16 Settings app and all of its subpages.
fn render_settings_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    let dark = screen_state.dark_mode;
    let bg_col = if dark {
        rgb(0, 0, 0)
    } else {
        rgb(242, 242, 247)
    };
    let card_col = if dark {
        rgb(28, 28, 30)
    } else {
        rgb(255, 255, 255)
    };
    let fg_col = if dark {
        rgb(255, 255, 255)
    } else {
        rgb(18, 18, 22)
    };
    let sub_col = rgb(142, 142, 147);
    let sep_col = if dark {
        rgb(56, 56, 58)
    } else {
        rgb(230, 230, 235)
    };

    fb.fill(bg_col);
    draw_iphone_x_status_bar(fb, fb_w, fb_h, !dark, screen_state);

    let card_x = 16;
    let card_w = (w - 32).max(160);

    match screen_state.settings_page.as_str() {
        "AppleID" => {
            draw_subpage_header(fb, fb_w, fb_h, "Settings", "Apple ID", dark);
            fill_circle(fb, fb_w, fb_h, w / 2, 108, 30, rgb(142, 152, 172));
            fill_circle(fb, fb_w, fb_h, w / 2, 100, 10, rgb(255, 255, 255));
            fill_rounded_rect(fb, fb_w, fb_h, w / 2 - 16, 114, 32, 14, 7, rgb(255, 255, 255));
            draw_text_centered(fb, fb_w, fb_h, 150, "iPhone X User", fg_col, 2, 2, false);
            draw_text_centered(fb, fb_w, fb_h, 174, "khanh@icloud.com", sub_col, 1, 2, false);

            let rows = [
                ("Name, Phone Numbers, Email", ""),
                ("Password & Security", "2FA On"),
                ("iCloud+", "50 GB (12.4 GB Used)"),
                ("Media & Purchases", "Active"),
                ("Find My", "On"),
                ("iPhone X (This iPhone 10,3)", "iOS 16.7.16"),
            ];
            fill_rounded_rect(
                fb,
                fb_w,
                fb_h,
                card_x,
                204,
                card_w,
                rows.len() as i32 * 38,
                10,
                card_col,
            );
            for (i, &(k, v)) in rows.iter().enumerate() {
                let ry = 204 + (i as i32) * 38;
                if i > 0 {
                    fill_rect(fb, fb_w, fb_h, card_x + 14, ry, card_w - 14, 1, sep_col);
                }
                draw_text_scaled(fb, fb_w, fb_h, card_x + 14, ry + 13, k, fg_col, 1, 2);
                draw_text_right(
                    fb,
                    fb_w,
                    fb_h,
                    card_x + card_w - 14,
                    ry + 13,
                    &format!("{} >", v),
                    sub_col,
                    1,
                    2,
                );
            }
        }
        "WiFi" => {
            draw_subpage_header(fb, fb_w, fb_h, "Settings", "Wi-Fi", dark);
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 86, card_w, 40, 10, card_col);
            draw_text_scaled(fb, fb_w, fb_h, card_x + 14, 100, "Wi-Fi", fg_col, 1, 2);
            draw_ios_switch(
                fb,
                fb_w,
                fb_h,
                card_x + card_w - 50,
                95,
                screen_state.wifi_enabled,
            );

            if screen_state.wifi_enabled {
                let net_name = if screen_state.wifi_network.is_empty() {
                    "SweetNet-5G"
                } else {
                    screen_state.wifi_network.as_str()
                };
                fill_rounded_rect(fb, fb_w, fb_h, card_x, 136, card_w, 40, 10, card_col);
                draw_text_semibold(
                    fb,
                    fb_w,
                    fb_h,
                    card_x + 14,
                    150,
                    &format!("v  {}", net_name),
                    rgb(0, 122, 255),
                    1,
                    2,
                    false,
                );
                draw_text_right(
                    fb,
                    fb_w,
                    fb_h,
                    card_x + card_w - 14,
                    150,
                    "Connected (i)",
                    sub_col,
                    1,
                    2,
                );

                draw_text_scaled(fb, fb_w, fb_h, card_x + 12, 188, "NETWORKS (TAP TO SWITCH)", sub_col, 1, 2);
                let nets = [
                    "SweetNet-5G",
                    "Viettel-5G-Ultra",
                    "VNPT-Fiber-Home",
                    "FPT-Telecom-5G",
                ];
                fill_rounded_rect(fb, fb_w, fb_h, card_x, 204, card_w, 160, 10, card_col);
                for (i, &n) in nets.iter().enumerate() {
                    let ry = 204 + (i as i32) * 40;
                    if i > 0 {
                        fill_rect(fb, fb_w, fb_h, card_x + 14, ry, card_w - 14, 1, sep_col);
                    }
                    let active = n == net_name;
                    draw_text_scaled(
                        fb,
                        fb_w,
                        fb_h,
                        card_x + 14,
                        ry + 14,
                        n,
                        if active { rgb(0, 122, 255) } else { fg_col },
                        1,
                        2,
                    );
                    draw_text_right(
                        fb,
                        fb_w,
                        fb_h,
                        card_x + card_w - 14,
                        ry + 14,
                        if active { "Active" } else { "5GHz" },
                        sub_col,
                        1,
                        2,
                    );
                }
            }
        }
        "Bluetooth" => {
            draw_subpage_header(fb, fb_w, fb_h, "Settings", "Bluetooth", dark);
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 86, card_w, 40, 10, card_col);
            draw_text_scaled(fb, fb_w, fb_h, card_x + 14, 100, "Bluetooth", fg_col, 1, 2);
            draw_ios_switch(
                fb,
                fb_w,
                fb_h,
                card_x + card_w - 50,
                95,
                screen_state.bluetooth_enabled,
            );
            draw_text_scaled(fb, fb_w, fb_h, card_x + 12, 142, "MY DEVICES", sub_col, 1, 2);
            let devs = [
                ("AirPods Pro (2nd Gen)", "Connected"),
                ("Apple Watch Series 8", "Connected"),
                ("MacBook Pro M2 Max", "Not Connected"),
            ];
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 160, card_w, 120, 10, card_col);
            for (i, &(d, st)) in devs.iter().enumerate() {
                let ry = 160 + (i as i32) * 40;
                if i > 0 {
                    fill_rect(fb, fb_w, fb_h, card_x + 14, ry, card_w - 14, 1, sep_col);
                }
                draw_text_scaled(fb, fb_w, fb_h, card_x + 14, ry + 14, d, fg_col, 1, 2);
                draw_text_right(fb, fb_w, fb_h, card_x + card_w - 14, ry + 14, st, sub_col, 1, 2);
            }
        }
        "Cellular" => {
            draw_subpage_header(fb, fb_w, fb_h, "Settings", "Cellular", dark);
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 86, card_w, 160, 10, card_col);
            draw_text_scaled(fb, fb_w, fb_h, card_x + 14, 100, "Cellular Data", fg_col, 1, 2);
            draw_ios_switch(
                fb,
                fb_w,
                fb_h,
                card_x + card_w - 50,
                95,
                screen_state.cellular_enabled,
            );
            let rows = [
                ("Cellular Data Options", "5G Auto >"),
                ("Personal Hotspot", "Off >"),
                ("Network Selection", "Viettel 54.0 >"),
            ];
            for (i, &(k, v)) in rows.iter().enumerate() {
                let ry = 126 + (i as i32) * 40;
                fill_rect(fb, fb_w, fb_h, card_x + 14, ry, card_w - 14, 1, sep_col);
                draw_text_scaled(fb, fb_w, fb_h, card_x + 14, ry + 14, k, fg_col, 1, 2);
                draw_text_right(fb, fb_w, fb_h, card_x + card_w - 14, ry + 14, v, sub_col, 1, 2);
            }
        }
        "General" => {
            draw_subpage_header(fb, fb_w, fb_h, "Settings", "General", dark);
            let rows = [
                ("About", "iPhone X >"),
                ("Software Update", "iOS 16.7.16 >"),
                ("AirDrop", "Everyone >"),
                ("iPhone Storage", "2.48 GB of 16.0 GB >"),
                ("Background App Refresh", "On >"),
                ("Date & Time", "24-Hour Time >"),
                ("Language & Region", "English / VN >"),
                ("VPN & Device Management", "Connected >"),
            ];
            fill_rounded_rect(
                fb,
                fb_w,
                fb_h,
                card_x,
                86,
                card_w,
                rows.len() as i32 * 36,
                10,
                card_col,
            );
            for (i, &(k, v)) in rows.iter().enumerate() {
                let ry = 86 + (i as i32) * 36;
                if i > 0 {
                    fill_rect(fb, fb_w, fb_h, card_x + 14, ry, card_w - 14, 1, sep_col);
                }
                draw_text_scaled(fb, fb_w, fb_h, card_x + 14, ry + 12, k, fg_col, 1, 2);
                draw_text_right(fb, fb_w, fb_h, card_x + card_w - 14, ry + 12, v, sub_col, 1, 2);
            }
        }
        "About" => {
            draw_subpage_header(fb, fb_w, fb_h, "General", "About", dark);
            let rows = [
                ("Name", "iPhone X"),
                ("iOS Version", "16.7.16 (20H392)"),
                ("Model Name", "iPhone X"),
                ("Model Number", "MQAC2LL/A (iPhone10,3)"),
                ("Board Config", "D22AP (sweet-virt)"),
                ("Serial Number", "F2LV809XJCLH"),
                ("Kernel", "Darwin 22.6.0 XNU-8796"),
                ("Total Disk", "16.0 GB (ios16-root)"),
                ("Available", "13.52 GB"),
                ("Wi-Fi Address", "52:54:00:12:34:56"),
                ("Modem Firmware", "8.50.04"),
            ];
            fill_rounded_rect(
                fb,
                fb_w,
                fb_h,
                card_x,
                86,
                card_w,
                rows.len() as i32 * 36,
                10,
                card_col,
            );
            for (i, &(k, v)) in rows.iter().enumerate() {
                let ry = 86 + (i as i32) * 36;
                if i > 0 {
                    fill_rect(fb, fb_w, fb_h, card_x + 14, ry, card_w - 14, 1, sep_col);
                }
                draw_text_scaled(fb, fb_w, fb_h, card_x + 14, ry + 12, k, fg_col, 1, 2);
                draw_text_right(fb, fb_w, fb_h, card_x + card_w - 14, ry + 12, v, sub_col, 1, 2);
            }
        }
        "SoftwareUpdate" => {
            draw_subpage_header(fb, fb_w, fb_h, "General", "Software Update", dark);
            render_squircle_icon(fb, fb_w, fb_h, w / 2 - 32, 160, 64, true, |canvas, cw, _ch| {
                draw_stock_icon_canvas(canvas, cw, "Settings");
            });
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                244,
                "iOS 16.7.16 (20H392)",
                fg_col,
                2,
                2,
                false,
            );
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                274,
                "iOS is up to date",
                sub_col,
                1,
                2,
                false,
            );
        }
        "Storage" => {
            draw_subpage_header(fb, fb_w, fb_h, "General", "iPhone Storage", dark);
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 86, card_w, 84, 10, card_col);
            draw_text_semibold(fb, fb_w, fb_h, card_x + 14, 102, "iPhone X", fg_col, 1, 2, false);
            draw_text_right(
                fb,
                fb_w,
                fb_h,
                card_x + card_w - 14,
                102,
                "2.48 GB of 16.0 GB Used",
                sub_col,
                1,
                2,
            );
            fill_rounded_rect(
                fb,
                fb_w,
                fb_h,
                card_x + 14,
                128,
                card_w - 28,
                16,
                6,
                rgb(218, 218, 226),
            );
            fill_rounded_rect(fb, fb_w, fb_h, card_x + 14, 128, 42, 16, 6, rgb(255, 59, 48));
            fill_rect(fb, fb_w, fb_h, card_x + 52, 128, 28, 16, rgb(0, 122, 255));
            fill_rect(fb, fb_w, fb_h, card_x + 80, 128, 24, 16, rgb(255, 149, 0));

            let apps = [
                ("iOS 16.7.16 System", "1.82 GB"),
                ("Photos & Camera", "312 MB"),
                ("Safari Cache", "148 MB"),
                ("Apple Music", "124 MB"),
                ("Messages", "76 MB"),
            ];
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 184, card_w, 190, 10, card_col);
            for (i, &(a, sz)) in apps.iter().enumerate() {
                let ry = 184 + (i as i32) * 38;
                if i > 0 {
                    fill_rect(fb, fb_w, fb_h, card_x + 14, ry, card_w - 14, 1, sep_col);
                }
                draw_text_scaled(fb, fb_w, fb_h, card_x + 14, ry + 13, a, fg_col, 1, 2);
                draw_text_right(fb, fb_w, fb_h, card_x + card_w - 14, ry + 13, sz, sub_col, 1, 2);
            }
        }
        "Display" => {
            draw_subpage_header(fb, fb_w, fb_h, "Settings", "Display & Brightness", dark);
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 96, card_w, 118, 10, card_col);
            // Light vs Dark preview cards
            fill_rounded_rect(fb, fb_w, fb_h, card_x + 36, 108, 92, 62, 8, rgb(235, 238, 245));
            draw_text_scaled(fb, fb_w, fb_h, card_x + 66, 178, "Light", fg_col, 1, 2);
            fill_circle(
                fb,
                fb_w,
                fb_h,
                card_x + 82,
                200,
                8,
                if !dark { rgb(0, 122, 255) } else { sub_col },
            );

            fill_rounded_rect(fb, fb_w, fb_h, card_x + 196, 108, 92, 62, 8, rgb(28, 30, 38));
            draw_text_scaled(fb, fb_w, fb_h, card_x + 228, 178, "Dark", fg_col, 1, 2);
            fill_circle(
                fb,
                fb_w,
                fb_h,
                card_x + 242,
                200,
                8,
                if dark { rgb(0, 122, 255) } else { sub_col },
            );

            // Brightness slider card
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 228, card_w, 54, 10, card_col);
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                card_x + 14,
                236,
                &format!("BRIGHTNESS ({}%)", screen_state.brightness),
                sub_col,
                1,
                2,
            );
            let sl_x = card_x + 28;
            let sl_w = card_w - 56;
            fill_rounded_rect(fb, fb_w, fb_h, sl_x, 260, sl_w, 6, 3, sep_col);
            let fill_w = (sl_w * (screen_state.brightness as i32)) / 100;
            fill_rounded_rect(fb, fb_w, fb_h, sl_x, 260, fill_w, 6, 3, rgb(0, 122, 255));
            fill_circle(fb, fb_w, fb_h, sl_x + fill_w, 263, 10, rgb(255, 255, 255));

            // True Tone & Night Shift toggles
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 294, card_w, 76, 10, card_col);
            draw_text_scaled(fb, fb_w, fb_h, card_x + 14, 306, "True Tone", fg_col, 1, 2);
            draw_ios_switch(
                fb,
                fb_w,
                fb_h,
                card_x + card_w - 50,
                301,
                screen_state.true_tone,
            );
            fill_rect(fb, fb_w, fb_h, card_x + 14, 332, card_w - 14, 1, sep_col);
            draw_text_scaled(fb, fb_w, fb_h, card_x + 14, 344, "Night Shift", fg_col, 1, 2);
            draw_ios_switch(
                fb,
                fb_w,
                fb_h,
                card_x + card_w - 50,
                339,
                screen_state.night_shift,
            );
        }
        "Wallpaper" => {
            draw_subpage_header(fb, fb_w, fb_h, "Settings", "Wallpaper", dark);
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                84,
                "Tap a Wallpaper Pair to Apply",
                sub_col,
                1,
                2,
                false,
            );
            fill_rounded_rect_gradient(
                fb,
                fb_w,
                fb_h,
                32,
                115,
                132,
                230,
                16,
                rgb(0, 150, 199),
                rgb(244, 162, 97),
            );
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                54,
                356,
                if !dark { "v iOS 16 Light" } else { "iOS 16 Light" },
                rgb(0, 122, 255),
                1,
                2,
                false,
            );

            fill_rounded_rect_gradient(
                fb,
                fb_w,
                fb_h,
                196,
                115,
                132,
                230,
                16,
                rgb(10, 18, 42),
                rgb(28, 53, 87),
            );
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                218,
                356,
                if dark { "v iOS 16 Dark" } else { "iOS 16 Dark" },
                rgb(0, 122, 255),
                1,
                2,
                false,
            );
        }
        "Battery" => {
            draw_subpage_header(fb, fb_w, fb_h, "Settings", "Battery", dark);
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 86, card_w, 76, 10, card_col);
            draw_text_scaled(fb, fb_w, fb_h, card_x + 14, 98, "Battery Percentage", fg_col, 1, 2);
            draw_ios_switch(fb, fb_w, fb_h, card_x + card_w - 50, 93, true);
            fill_rect(fb, fb_w, fb_h, card_x + 14, 124, card_w - 14, 1, sep_col);
            draw_text_scaled(fb, fb_w, fb_h, card_x + 14, 136, "Low Power Mode", fg_col, 1, 2);
            draw_ios_switch(
                fb,
                fb_w,
                fb_h,
                card_x + card_w - 50,
                131,
                screen_state.low_power_mode,
            );

            fill_rounded_rect(fb, fb_w, fb_h, card_x, 176, card_w, 40, 10, card_col);
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                card_x + 14,
                190,
                "Battery Health & Charging",
                fg_col,
                1,
                2,
            );
            draw_text_right(
                fb,
                fb_w,
                fb_h,
                card_x + card_w - 14,
                190,
                "100% >",
                sub_col,
                1,
                2,
            );

            // 24-Hour Battery Level Chart
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 230, card_w, 120, 10, card_col);
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                card_x + 14,
                242,
                "LAST 24 HOURS — 100% CHARGED",
                sub_col,
                1,
                2,
            );
            let bar_col = if screen_state.low_power_mode {
                rgb(255, 204, 0)
            } else {
                rgb(52, 199, 89)
            };
            for b in 0..16i32 {
                let bh = 42 + ((b * 7) % 28);
                fill_rounded_rect(
                    fb,
                    fb_w,
                    fb_h,
                    card_x + 18 + b * 18,
                    336 - bh,
                    10,
                    bh,
                    3,
                    bar_col,
                );
            }
        }
        "Sounds" | "Focus" | "Notifications" | "ScreenTime" | "ControlCenter" => {
            draw_subpage_header(
                fb,
                fb_w,
                fb_h,
                "Settings",
                &screen_state.settings_page,
                dark,
            );
            fill_rounded_rect(fb, fb_w, fb_h, card_x, 86, card_w, 96, 10, card_col);
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                card_x + 14,
                100,
                "Do Not Disturb / Alerts",
                fg_col,
                1,
                2,
            );
            draw_ios_switch(
                fb,
                fb_w,
                fb_h,
                card_x + card_w - 50,
                95,
                screen_state.focus_dnd,
            );
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                card_x + 14,
                136,
                &format!("Ringer & Haptics Volume: {}%", screen_state.volume),
                sub_col,
                1,
                2,
            );
            let sl_x = card_x + 18;
            let sl_w = card_w - 36;
            fill_rounded_rect(fb, fb_w, fb_h, sl_x, 158, sl_w, 6, 3, sep_col);
            let fw = (sl_w * (screen_state.volume as i32)) / 100;
            fill_rounded_rect(fb, fb_w, fb_h, sl_x, 158, fw, 6, 3, rgb(0, 122, 255));
        }
        _ => {
            // Main Settings Scrollable List
            let mut cur_y = 42 - screen_state.scroll_y;
            draw_text_semibold(fb, fb_w, fb_h, card_x, cur_y, "Settings", fg_col, 2, 2, false);
            cur_y += 24;

            // Search bar
            fill_rounded_rect(
                fb,
                fb_w,
                fb_h,
                card_x,
                cur_y,
                card_w,
                28,
                8,
                if dark {
                    rgb(44, 44, 46)
                } else {
                    rgb(227, 227, 233)
                },
            );
            draw_ring(fb, fb_w, fb_h, card_x + 14, cur_y + 13, 4, 5, sub_col);
            draw_line(
                fb,
                fb_w,
                fb_h,
                card_x + 17,
                cur_y + 16,
                card_x + 20,
                cur_y + 19,
                1,
                sub_col,
            );
            draw_text_scaled(fb, fb_w, fb_h, card_x + 26, cur_y + 9, "Search", sub_col, 1, 2);
            cur_y += 38;

            // Apple ID Profile Card
            fill_rounded_rect(fb, fb_w, fb_h, card_x, cur_y, card_w, 58, 10, card_col);
            let av_cx = card_x + 28;
            let av_cy = cur_y + 29;
            fill_circle(fb, fb_w, fb_h, av_cx, av_cy, 19, rgb(164, 174, 192));
            fill_circle(fb, fb_w, fb_h, av_cx, av_cy - 5, 6, rgb(255, 255, 255));
            fill_rounded_rect(
                fb,
                fb_w,
                fb_h,
                av_cx - 10,
                av_cy + 4,
                20,
                10,
                5,
                rgb(255, 255, 255),
            );
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                card_x + 56,
                cur_y + 13,
                "iPhone X User",
                fg_col,
                1,
                2,
                false,
            );
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                card_x + 56,
                cur_y + 31,
                "Apple ID, iCloud+, Media & Purchases",
                sub_col,
                1,
                2,
            );
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                card_x + card_w - 16,
                cur_y + 23,
                ">",
                sub_col,
                1,
                2,
            );
            cur_y += 68;

            let wifi_detail = if !screen_state.wifi_enabled || screen_state.airplane_mode {
                "Off"
            } else if screen_state.wifi_network.is_empty() {
                "SweetNet-5G"
            } else {
                screen_state.wifi_network.as_str()
            };
            let bt_detail = if screen_state.bluetooth_enabled {
                "On"
            } else {
                "Off"
            };
            let cell_detail = if screen_state.airplane_mode || !screen_state.cellular_enabled {
                "Off"
            } else {
                "Viettel 5G"
            };

            let draw_group =
                |fb: &mut [u32], start_y: i32, rows: &[(&str, &str, u32, Option<bool>)]| -> i32 {
                    let row_h = 36;
                    let total_h = (rows.len() as i32) * row_h;
                    fill_rounded_rect(fb, fb_w, fb_h, card_x, start_y, card_w, total_h, 10, card_col);
                    for (i, &(title, detail, badge_col, toggle_opt)) in rows.iter().enumerate() {
                        let ry = start_y + (i as i32) * row_h;
                        if i > 0 {
                            fill_rect(fb, fb_w, fb_h, card_x + 42, ry, card_w - 42, 1, sep_col);
                        }
                        draw_settings_row_badge(
                            fb,
                            fb_w,
                            fb_h,
                            card_x + 10,
                            ry + 6,
                            badge_col,
                            title,
                        );
                        draw_text_scaled(fb, fb_w, fb_h, card_x + 42, ry + 12, title, fg_col, 1, 2);

                        if let Some(is_on) = toggle_opt {
                            draw_ios_switch(fb, fb_w, fb_h, card_x + card_w - 48, ry + 7, is_on);
                        } else {
                            let chev_x = card_x + card_w - 16;
                            draw_text_scaled(fb, fb_w, fb_h, chev_x, ry + 12, ">", sub_col, 1, 2);
                            if !detail.is_empty() {
                                draw_text_right(
                                    fb,
                                    fb_w,
                                    fb_h,
                                    chev_x - 4,
                                    ry + 12,
                                    detail,
                                    sub_col,
                                    1,
                                    2,
                                );
                            }
                        }
                    }
                    start_y + total_h + 12
                };

            let group1 = [
                (
                    "Airplane Mode",
                    "",
                    rgb(255, 149, 0),
                    Some(screen_state.airplane_mode),
                ),
                ("Wi-Fi", wifi_detail, rgb(0, 122, 255), None),
                ("Bluetooth", bt_detail, rgb(0, 122, 255), None),
                ("Cellular", cell_detail, rgb(52, 199, 89), None),
            ];
            cur_y = draw_group(fb, cur_y, &group1);

            let group2 = [
                ("Notifications", "Scheduled", rgb(255, 59, 48), None),
                ("Sounds & Haptics", "", rgb(255, 45, 85), None),
                (
                    "Focus",
                    if screen_state.focus_dnd { "DND" } else { "" },
                    rgb(88, 86, 214),
                    None,
                ),
                ("Screen Time", "", rgb(88, 86, 214), None),
            ];
            cur_y = draw_group(fb, cur_y, &group2);

            let group3 = [
                (
                    "General",
                    "iOS 16.7.16 (20H392) - iPhone10,3",
                    rgb(142, 142, 147),
                    None,
                ),
                ("Control Center", "", rgb(142, 142, 147), None),
                (
                    "Display & Brightness",
                    if dark { "Dark" } else { "Light" },
                    rgb(0, 122, 255),
                    None,
                ),
                ("Wallpaper", "", rgb(48, 176, 199), None),
                (
                    "Battery",
                    if screen_state.low_power_mode {
                        "Low Power"
                    } else {
                        "100%"
                    },
                    rgb(52, 199, 89),
                    None,
                ),
            ];
            let _ = draw_group(fb, cur_y, &group3);
        }
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, !dark);
}

/// Renders the interactive iOS 16 Phone app (Keypad, Active Call Screen, Recents, Contacts).
fn render_phone_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    let h = fb_h as i32;

    if screen_state.phone_in_call {
        draw_ios16_wallpaper(fb, fb_w, fb_h, true, screen_state.frame_tick);
        blur_rounded_rect(
            fb,
            fb_w,
            fb_h,
            0,
            0,
            w,
            h,
            0,
            14,
            rgba(12, 18, 32, 195),
            0,
        );
        draw_iphone_x_status_bar(fb, fb_w, fb_h, false, screen_state);

        let target = if screen_state.phone_call_target.is_empty() {
            "198 (Viettel Care)"
        } else {
            screen_state.phone_call_target.as_str()
        };
        draw_text_centered(fb, fb_w, fb_h, 78, target, rgb(255, 255, 255), 2, 2, false);
        let secs = screen_state
            .frame_tick
            .saturating_sub(screen_state.phone_call_start_tick)
            / 60;
        let dur_str = format!("{:02}:{:02}", secs / 60, secs % 60);
        draw_text_centered(
            fb,
            fb_w,
            fb_h,
            110,
            &dur_str,
            rgba(255, 255, 255, 210),
            1,
            2,
            false,
        );

        let call_btns = [
            ("mute", screen_state.phone_muted),
            ("keypad", false),
            ("speaker", screen_state.phone_speaker),
            ("add call", false),
            ("FaceTime", false),
            ("contacts", false),
        ];
        for (i, &(lbl, active)) in call_btns.iter().enumerate() {
            let col = (i as i32) % 3;
            let row = (i as i32) / 3;
            let cx = 86 + col * 94;
            let cy = 270 + row * 100;
            fill_circle(
                fb,
                fb_w,
                fb_h,
                cx,
                cy,
                32,
                if active {
                    rgb(255, 255, 255)
                } else {
                    rgba(255, 255, 255, 48)
                },
            );
            draw_text_centered(fb, 0, 0, 0, "", 0, 1, 1, false);
            let lw = measure_text_px(lbl, 1);
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                cx - lw / 2,
                cy + 40,
                lbl,
                rgb(255, 255, 255),
                1,
                2,
            );
        }

        // Red End Call Circle
        fill_circle(fb, fb_w, fb_h, w / 2, 512, 36, rgb(255, 59, 48));
        draw_line(
            fb,
            fb_w,
            fb_h,
            w / 2 - 12,
            512,
            w / 2 + 12,
            512,
            5,
            rgb(255, 255, 255),
        );
        draw_home_indicator(fb, fb_w, fb_h, screen_state, false);
        return;
    }

    fb.fill(rgb(255, 255, 255));
    draw_iphone_x_status_bar(fb, fb_w, fb_h, true, screen_state);

    let tab = if screen_state.initialized {
        screen_state.phone_tab
    } else {
        3
    };

    if tab == 3 {
        // Dialed number buffer at top of Keypad
        if !screen_state.phone_dial_buf.is_empty() {
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                68,
                &screen_state.phone_dial_buf,
                rgb(18, 18, 22),
                2,
                2,
                false,
            );
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                98,
                "Add Number",
                rgb(0, 122, 255),
                1,
                2,
                false,
            );
        }

        let keys: [(&str, &str); 12] = [
            ("1", ""),
            ("2", "ABC"),
            ("3", "DEF"),
            ("4", "GHI"),
            ("5", "JKL"),
            ("6", "MNO"),
            ("7", "PQRS"),
            ("8", "TUV"),
            ("9", "WXYZ"),
            ("*", ""),
            ("0", "+"),
            ("#", ""),
        ];
        for (i, &(num, sub)) in keys.iter().enumerate() {
            let col = (i as i32) % 3;
            let row = (i as i32) / 3;
            let cx = 92 + col * 88;
            let cy = 165 + row * 80;
            fill_circle(fb, fb_w, fb_h, cx, cy, 34, rgb(229, 229, 234));
            let ch = num.as_bytes()[0];
            if ch.is_ascii_digit() {
                let nw = measure_sdf_numerals(num, 24, 0);
                draw_sdf_numeral(
                    fb,
                    fb_w,
                    fb_h,
                    cx - nw / 2,
                    cy - 15,
                    ch,
                    24,
                    1.6,
                    rgb(20, 20, 24),
                );
            } else {
                let nw = measure_text_px(num, 2);
                draw_text_semibold(
                    fb,
                    fb_w,
                    fb_h,
                    cx - nw / 2,
                    cy - 10,
                    num,
                    rgb(20, 20, 24),
                    2,
                    2,
                    false,
                );
            }
            if !sub.is_empty() {
                let sw = measure_text_px(sub, 1);
                draw_text_scaled(
                    fb,
                    fb_w,
                    fb_h,
                    cx - sw / 2,
                    cy + 12,
                    sub,
                    rgb(90, 90, 98),
                    1,
                    2,
                );
            }
        }

        // Green Call Button
        let call_cy = 492;
        fill_circle(fb, fb_w, fb_h, w / 2, call_cy, 34, rgb(52, 199, 89));
        draw_line(
            fb,
            fb_w,
            fb_h,
            w / 2 - 8,
            call_cy - 8,
            w / 2 + 8,
            call_cy + 8,
            5,
            rgb(255, 255, 255),
        );
        if !screen_state.phone_dial_buf.is_empty() {
            fill_rounded_rect(fb, fb_w, fb_h, 256, call_cy - 12, 34, 24, 6, rgb(215, 215, 222));
            draw_text_semibold(fb, fb_w, fb_h, 266, call_cy - 6, "x", rgb(60, 60, 68), 1, 2, false);
        }
    } else {
        let title = ["Favorites", "Recents", "Contacts", "Keypad", "Voicemail"][tab as usize];
        draw_text_semibold(fb, fb_w, fb_h, 20, 48, title, rgb(18, 18, 22), 2, 2, false);
        draw_text_scaled(
            fb,
            fb_w,
            fb_h,
            20,
            76,
            "Tap any entry below to place a live call",
            rgb(142, 142, 147),
            1,
            2,
        );
        let default_list = [
            ("Khanh (iPhone X)", "Mobile • +84 98 888 9999"),
            ("Apple Support", "Cupertino • 1-800-MY-APPLE"),
            ("Viettel 198", "Carrier Customer Care"),
            ("Team ios-sweet", "FaceTime Audio • Online"),
        ];
        for i in 0..4usize {
            let (name, sub) = screen_state
                .phone_recents
                .get(i)
                .map(|(a, b)| (a.as_str(), b.as_str()))
                .unwrap_or(default_list[i]);
            let ry = 104 + (i as i32) * 56;
            fill_circle(fb, fb_w, fb_h, 38, ry + 22, 18, rgb(0, 122, 255));
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                68,
                ry + 8,
                name,
                rgb(18, 18, 22),
                1,
                2,
                false,
            );
            draw_text_scaled(fb, fb_w, fb_h, 68, ry + 26, sub, rgb(142, 142, 147), 1, 2);
            draw_text_right(
                fb,
                fb_w,
                fb_h,
                w - 22,
                ry + 16,
                "Call >",
                rgb(52, 199, 89),
                1,
                2,
            );
            fill_rect(fb, fb_w, fb_h, 68, ry + 52, w - 84, 1, rgb(232, 232, 238));
        }
    }

    // Bottom Tab Bar
    let tab_y = h - 56;
    fill_rect(fb, fb_w, fb_h, 0, tab_y, w, 1, rgb(218, 218, 224));
    let tabs = ["Favorites", "Recents", "Contacts", "Keypad", "Voicemail"];
    let tab_w = w / 5;
    for (i, &t_name) in tabs.iter().enumerate() {
        let tx = (i as i32) * tab_w + tab_w / 2;
        let col = if i as u8 == tab {
            rgb(0, 122, 255)
        } else {
            rgb(142, 142, 147)
        };
        let tw = measure_text_px(t_name, 1);
        draw_text_scaled(fb, fb_w, fb_h, tx - tw / 2, tab_y + 18, t_name, col, 1, 2);
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, true);
}

/// Renders the interactive iOS 16 Calculator app with live expression display.
fn render_calculator_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    fb.fill(rgb(0, 0, 0));
    draw_iphone_x_status_bar(fb, fb_w, fb_h, false, screen_state);

    let disp = if screen_state.calc_display.is_empty() {
        "0"
    } else {
        screen_state.calc_display.as_str()
    };

    // Render crisp calculator display right-aligned at y = 165
    let num_h = if disp.len() > 7 { 40 } else { 58 };
    if disp.bytes().all(|b| b.is_ascii_digit()) {
        let total_w = measure_sdf_numerals(disp, num_h, 2);
        let mut cx = (w - 28 - total_w).max(16);
        for b in disp.bytes() {
            let cw = draw_sdf_numeral(
                fb,
                fb_w,
                fb_h,
                cx,
                165,
                b,
                num_h,
                3.5,
                rgb(255, 255, 255),
            );
            cx += cw + 2;
        }
    } else {
        draw_text_right(fb, fb_w, fb_h, w - 28, 185, disp, rgb(255, 255, 255), 3, 3);
    }

    let calc_rows: [[(&str, u32, u32); 4]; 5] = [
        [
            ("AC", rgb(165, 165, 165), rgb(0, 0, 0)),
            ("+/-", rgb(165, 165, 165), rgb(0, 0, 0)),
            ("%", rgb(165, 165, 165), rgb(0, 0, 0)),
            ("/", rgb(255, 159, 10), rgb(255, 255, 255)),
        ],
        [
            ("7", rgb(51, 51, 51), rgb(255, 255, 255)),
            ("8", rgb(51, 51, 51), rgb(255, 255, 255)),
            ("9", rgb(51, 51, 51), rgb(255, 255, 255)),
            ("x", rgb(255, 159, 10), rgb(255, 255, 255)),
        ],
        [
            ("4", rgb(51, 51, 51), rgb(255, 255, 255)),
            ("5", rgb(51, 51, 51), rgb(255, 255, 255)),
            ("6", rgb(51, 51, 51), rgb(255, 255, 255)),
            ("-", rgb(255, 159, 10), rgb(255, 255, 255)),
        ],
        [
            ("1", rgb(51, 51, 51), rgb(255, 255, 255)),
            ("2", rgb(51, 51, 51), rgb(255, 255, 255)),
            ("3", rgb(51, 51, 51), rgb(255, 255, 255)),
            ("+", rgb(255, 159, 10), rgb(255, 255, 255)),
        ],
        [
            ("0", rgb(51, 51, 51), rgb(255, 255, 255)),
            (".", rgb(51, 51, 51), rgb(255, 255, 255)),
            ("=", rgb(255, 159, 10), rgb(255, 255, 255)),
            ("", 0, 0),
        ],
    ];

    for (r_idx, row) in calc_rows.iter().enumerate() {
        let cy = 285 + (r_idx as i32) * 86;
        for (c_idx, &(lbl, mut bg_c, mut fg_c)) in row.iter().enumerate() {
            if lbl.is_empty() {
                continue;
            }
            // Invert active operator button just like real iOS Calculator
            let is_pending = matches!(
                (lbl, screen_state.calc_pending_op),
                ("+", Some('+'))
                    | ("-", Some('-'))
                    | ("x", Some('*'))
                    | ("/", Some('/'))
            );
            if is_pending && screen_state.calc_replace_next {
                bg_c = rgb(255, 255, 255);
                fg_c = rgb(255, 159, 10);
            }
            let cx = 52 + (c_idx as i32) * 85;
            fill_circle(fb, fb_w, fb_h, cx, cy, 36, bg_c);
            let lw = measure_text_px(lbl, 2);
            draw_text_semibold(fb, fb_w, fb_h, cx - lw / 2, cy - 8, lbl, fg_c, 2, 2, false);
        }
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, false);
}

/// Renders the interactive iOS 16 Safari browser (Start Page + Live Webpages + Address Bar + Keyboard).
fn render_safari_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    let h = fb_h as i32;

    if screen_state.safari_url.is_empty() {
        draw_ios16_wallpaper(
            fb,
            fb_w,
            fb_h,
            screen_state.dark_mode,
            screen_state.frame_tick,
        );
        blur_rounded_rect(
            fb,
            fb_w,
            fb_h,
            0,
            0,
            w,
            h,
            0,
            12,
            rgba(235, 240, 250, 195),
            0,
        );
        draw_iphone_x_status_bar(fb, fb_w, fb_h, true, screen_state);

        let margin = 20;
        let mut cur_y = 54;
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            margin,
            cur_y,
            "Favorites",
            rgb(24, 24, 30),
            2,
            2,
            false,
        );
        cur_y += 32;

        let favs: [(&str, u32, &str); 6] = [
            ("Apple", rgb(28, 28, 30), "A"),
            ("Google", rgb(66, 133, 244), "G"),
            ("VnExpress", rgb(158, 34, 78), "VN"),
            ("Wikipedia", rgb(255, 255, 255), "W"),
            ("GitHub", rgb(36, 41, 47), "GH"),
            ("YouTube", rgb(255, 0, 0), "YT"),
        ];
        let cols = 4;
        let cell_w = (w - 2 * margin) / cols;
        for (i, &(name, bg_col, sym)) in favs.iter().enumerate() {
            let col = (i as i32) % cols;
            let row = (i as i32) / cols;
            let cx = margin + col * cell_w + cell_w / 2;
            let ix = cx - 26;
            let iy = cur_y + row * 82;
            render_squircle_icon(fb, fb_w, fb_h, ix, iy, 52, true, |canvas, cw, _ch| {
                canvas.fill(bg_col);
                let txt_col = if bg_col == rgb(255, 255, 255) {
                    rgb(24, 24, 28)
                } else {
                    rgb(255, 255, 255)
                };
                let sw = measure_text_px(sym, 2);
                draw_text_semibold(canvas, cw, cw, 26 - sw / 2, 18, sym, txt_col, 2, 2, false);
            });
            let nw = measure_text_px(name, 1);
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                cx - nw / 2,
                iy + 58,
                name,
                rgb(40, 42, 50),
                1,
                2,
            );
        }

        cur_y += 180;
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            margin,
            cur_y,
            "Privacy Report",
            rgb(24, 24, 30),
            1,
            2,
            false,
        );
        cur_y += 20;
        fill_rounded_rect(
            fb,
            fb_w,
            fb_h,
            margin,
            cur_y,
            w - 2 * margin,
            68,
            14,
            rgba(255, 255, 255, 215),
        );
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            margin + 16,
            cur_y + 16,
            "14",
            rgb(52, 199, 89),
            2,
            2,
            false,
        );
        draw_text_scaled(
            fb,
            fb_w,
            fb_h,
            margin + 54,
            cur_y + 18,
            "Tap any Favorite icon above to browse",
            rgb(48, 50, 58),
            1,
            2,
        );
        draw_text_scaled(
            fb,
            fb_w,
            fb_h,
            margin + 54,
            cur_y + 34,
            "or tap the bottom URL bar to search.",
            rgb(48, 50, 58),
            1,
            2,
        );
    } else {
        // Active Webpage View
        fb.fill(rgb(255, 255, 255));
        draw_iphone_x_status_bar(fb, fb_w, fb_h, true, screen_state);
        let url = &screen_state.safari_url;
        let sy = screen_state.scroll_y;

        if url.contains("apple.com") {
            fill_rect(fb, fb_w, fb_h, 0, 36, w, 36, rgb(24, 24, 28));
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                48,
                "Apple  •  iPhone X  •  iOS 16.7.16",
                rgb(255, 255, 255),
                1,
                2,
                false,
            );
            fill_rounded_rect_gradient(
                fb,
                fb_w,
                fb_h,
                16,
                90 - sy,
                w - 32,
                190,
                18,
                rgb(18, 28, 56),
                rgb(0, 122, 255),
            );
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                130 - sy,
                "iPhone X",
                rgb(255, 255, 255),
                2,
                2,
                false,
            );
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                164 - sy,
                "Say hello to the future.",
                rgb(220, 235, 255),
                1,
                2,
                false,
            );
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                192 - sy,
                "5.8-inch Super Retina OLED • A11 Bionic",
                rgb(180, 215, 255),
                1,
                2,
                false,
            );
        } else if url.contains("vnexpress") {
            fill_rect(fb, fb_w, fb_h, 0, 36, w, 38, rgb(158, 34, 78));
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                49,
                "VNEXPRESS — Bao tieng Viet nhieu nguoi xem",
                rgb(255, 255, 255),
                1,
                2,
                false,
            );
            let news = [
                (
                    "Cong nghe: Chay thu nghiem iOS 16.7.16 tren Redmi Note 10 Pro",
                    "He thong dat 60 FPS voi day du ung dung tuong tac muot ma.",
                ),
                (
                    "Apple cap nhat bao mat iOS 16.7.16 (20H392) cho iPhone X",
                    "Ban build D22AP chinh thuc ho tro WebKit va Metal.",
                ),
                (
                    "Mang 5G Viettel phu song rong khap 63 tinh thanh",
                    "Toc do tai xuong vuot moc 1 Gbps tren thiet bi di dong.",
                ),
            ];
            for (i, &(hl, desc)) in news.iter().enumerate() {
                let ny = 92 + (i as i32) * 110 - sy;
                fill_rounded_rect(fb, fb_w, fb_h, 16, ny, w - 32, 96, 12, rgb(244, 245, 250));
                draw_text_semibold(fb, fb_w, fb_h, 28, ny + 18, hl, rgb(158, 34, 78), 1, 2, false);
                draw_text_scaled(fb, fb_w, fb_h, 28, ny + 44, desc, rgb(60, 62, 70), 1, 2);
            }
        } else {
            fill_rect(fb, fb_w, fb_h, 0, 36, w, 38, rgb(242, 244, 248));
            draw_text_centered(fb, fb_w, fb_h, 49, url, rgb(0, 122, 255), 1, 2, false);
            for i in 0..4i32 {
                let ry = 92 + i * 116 - sy;
                fill_rounded_rect(fb, fb_w, fb_h, 16, ry, w - 32, 100, 12, rgb(245, 247, 252));
                draw_text_semibold(
                    fb,
                    fb_w,
                    fb_h,
                    28,
                    ry + 18,
                    &format!("Web Result #{} — {}", i + 1, url),
                    rgb(26, 13, 171),
                    1,
                    2,
                    false,
                );
                draw_text_scaled(
                    fb,
                    fb_w,
                    fb_h,
                    28,
                    ry + 44,
                    "Interactive iOS 16.7.16 Mobile Safari WebKit Engine.",
                    rgb(70, 72, 80),
                    1,
                    2,
                );
                draw_text_scaled(
                    fb,
                    fb_w,
                    fb_h,
                    28,
                    ry + 66,
                    "Tap '<' below to return to Favorites Start Page.",
                    rgb(120, 124, 132),
                    1,
                    2,
                );
            }
        }
    }

    // Bottom Floating Address Bar + Navigation Toolbar
    let bar_y = if screen_state.safari_editing_url {
        466
    } else {
        h - 108
    };
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        14,
        bar_y,
        w - 28,
        38,
        12,
        rgb(255, 255, 255),
    );
    draw_text_scaled(fb, fb_w, fb_h, 24, bar_y + 13, "AA", rgb(80, 82, 90), 1, 2);
    let addr_txt = if screen_state.safari_editing_url {
        if screen_state.safari_input_buf.is_empty() {
            "Type URL or search query..."
        } else {
            screen_state.safari_input_buf.as_str()
        }
    } else if screen_state.safari_url.is_empty() {
        "Search or enter website name"
    } else {
        screen_state.safari_url.as_str()
    };
    draw_text_centered(
        fb,
        fb_w,
        fb_h,
        bar_y + 13,
        addr_txt,
        rgb(60, 62, 70),
        1,
        2,
        false,
    );

    if screen_state.safari_editing_url {
        draw_ios_keyboard(fb, fb_w, fb_h, screen_state.dark_mode, "Go");
    } else {
        let nav_y = h - 54;
        let nav_items = ["<", ">", "[+]", "Book", "Tabs"];
        for (i, &it) in nav_items.iter().enumerate() {
            let nx = 36 + (i as i32) * 72;
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                nx - 8,
                nav_y,
                it,
                rgb(0, 122, 255),
                1,
                2,
                false,
            );
        }
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, true);
}
/// Renders the interactive iOS 16 Messages (iMessage) app.
fn render_messages_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    fb.fill(rgb(255, 255, 255));
    draw_iphone_x_status_bar(fb, fb_w, fb_h, true, screen_state);

    if let Some(chat_idx) = screen_state.messages_active_chat {
        let (contact, _, msgs) = screen_state
            .messages_threads
            .get(chat_idx)
            .cloned()
            .unwrap_or_else(|| {
                (
                    "Apple".to_string(),
                    "9:41 AM".to_string(),
                    vec![(false, "Welcome to iMessage on iOS 16.7.16!".to_string())],
                )
            });

        draw_subpage_header(fb, fb_w, fb_h, "Messages", &contact, false);
        fill_rect(fb, fb_w, fb_h, 0, 72, w, 1, rgb(226, 226, 232));

        let mut cur_y = 88;
        let start_m = msgs.len().saturating_sub(5);
        for (is_me, txt) in &msgs[start_m..] {
            let bw = (measure_text_px(txt, 1) + 24).clamp(80, 265);
            let bx = if *is_me { w - 16 - bw } else { 16 };
            let bg = if *is_me {
                rgb(0, 122, 255)
            } else {
                rgb(229, 229, 234)
            };
            let fg = if *is_me {
                rgb(255, 255, 255)
            } else {
                rgb(18, 18, 22)
            };
            fill_rounded_rect(fb, fb_w, fb_h, bx, cur_y, bw, 36, 16, bg);
            let display_txt = if txt.len() > 38 { &txt[..38] } else { txt.as_str() };
            draw_text_scaled(fb, fb_w, fb_h, bx + 12, cur_y + 12, display_txt, fg, 1, 2);
            cur_y += 46;
        }

        // iMessage Input Bar + Blue Send Arrow
        fill_rounded_rect(
            fb,
            fb_w,
            fb_h,
            14,
            464,
            w - 64,
            36,
            18,
            rgb(238, 238, 244),
        );
        let draft = if screen_state.messages_input_buf.is_empty() {
            "iMessage (tap keys below)"
        } else {
            screen_state.messages_input_buf.as_str()
        };
        draw_text_scaled(fb, fb_w, fb_h, 28, 476, draft, rgb(80, 82, 90), 1, 2);
        fill_circle(fb, fb_w, fb_h, w - 30, 482, 16, rgb(0, 122, 255));
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            w - 34,
            476,
            "^",
            rgb(255, 255, 255),
            1,
            2,
            false,
        );

        draw_ios_keyboard(fb, fb_w, fb_h, false, "Send");
    } else {
        draw_text_semibold(fb, fb_w, fb_h, 20, 48, "Messages", rgb(18, 18, 22), 2, 2, false);
        fill_rounded_rect(fb, fb_w, fb_h, 20, 76, w - 40, 28, 8, rgb(236, 236, 242));
        draw_text_scaled(fb, fb_w, fb_h, 34, 84, "Search", rgb(142, 142, 147), 1, 2);

        let fallback = [
            (
                "Apple",
                "9:40 AM",
                "All stock apps, Settings, Phone, Safari ready.",
            ),
            (
                "Khanh (iPhone X)",
                "9:35 AM",
                "Da fix full icon vector + tat ca ung dung bam duoc!",
            ),
            (
                "Viettel",
                "Yesterday",
                "Quy khach dang su dung goi 5G Viettel toc do cao.",
            ),
            (
                "Team ios-sweet",
                "Monday",
                "SpringBoard 60 FPS running smoothly!",
            ),
        ];

        for i in 0..4usize {
            let (name, time_s, preview) = if let Some((n, t, m)) = screen_state.messages_threads.get(i) {
                let last = m.last().map(|(_, s)| s.as_str()).unwrap_or("");
                (n.as_str(), t.as_str(), last)
            } else {
                fallback[i]
            };
            let ry = 116 + (i as i32) * 64;
            fill_circle(fb, fb_w, fb_h, 38, ry + 26, 20, rgb(148, 162, 188));
            draw_text_semibold(fb, fb_w, fb_h, 68, ry + 10, name, rgb(18, 18, 22), 1, 2, false);
            draw_text_right(fb, fb_w, fb_h, w - 20, ry + 10, time_s, rgb(142, 142, 147), 1, 2);
            let short_p = if preview.len() > 36 {
                &preview[..36]
            } else {
                preview
            };
            draw_text_scaled(fb, fb_w, fb_h, 68, ry + 30, short_p, rgb(120, 122, 130), 1, 2);
            fill_rect(fb, fb_w, fb_h, 68, ry + 58, w - 84, 1, rgb(232, 232, 238));
        }
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, true);
}

/// Renders the interactive iOS 16 Camera app with live viewfinder, flash, and shutter.
fn render_camera_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    let h = fb_h as i32;
    fb.fill(rgb(0, 0, 0));

    // Viewfinder window (y: 88..620)
    let vf_h = (fb_h - 268).max(1);
    draw_ios16_wallpaper(
        &mut fb[88 * fb_w..(88 + vf_h) * fb_w],
        fb_w,
        vf_h,
        screen_state.camera_front,
        screen_state.frame_tick,
    );

    // Rule-of-thirds camera grid + yellow autofocus reticle
    let gx1 = w / 3;
    let gx2 = (w * 2) / 3;
    fill_rect(fb, fb_w, fb_h, gx1, 88, 1, vf_h as i32, rgba(255, 255, 255, 55));
    fill_rect(fb, fb_w, fb_h, gx2, 88, 1, vf_h as i32, rgba(255, 255, 255, 55));
    fill_rect(fb, fb_w, fb_h, 0, 265, w, 1, rgba(255, 255, 255, 55));
    fill_rect(fb, fb_w, fb_h, 0, 442, w, 1, rgba(255, 255, 255, 55));
    draw_ring(fb, fb_w, fb_h, w / 2, 340, 24, 26, rgb(255, 204, 0));

    // White shutter flash effect when photo was just captured
    if screen_state.camera_flash_tick > 0
        && screen_state.frame_tick.saturating_sub(screen_state.camera_flash_tick) <= 6
    {
        fill_rect(fb, fb_w, fb_h, 0, 88, w, vf_h as i32, rgba(255, 255, 255, 220));
    }

    draw_iphone_x_status_bar(fb, fb_w, fb_h, false, screen_state);
    draw_text_centered(
        fb,
        fb_w,
        fb_h,
        54,
        if screen_state.camera_front {
            "Front TrueDepth Camera • 1x"
        } else {
            "Dual 12MP Wide • 1x   2x"
        },
        rgb(255, 204, 0),
        1,
        2,
        false,
    );

    // Camera Mode Selector
    let modes = ["VIDEO", "PHOTO", "PORTRAIT", "PANO"];
    let cur_mode = if screen_state.initialized {
        screen_state.camera_mode as usize
    } else {
        1
    };
    for (i, &m) in modes.iter().enumerate() {
        let mx = 42 + (i as i32) * 78;
        let col = if i == cur_mode {
            rgb(255, 204, 0)
        } else {
            rgb(220, 220, 228)
        };
        draw_text_semibold(fb, fb_w, fb_h, mx, h - 150, m, col, 1, 2, false);
    }

    // Bottom-left Photo Thumbnail (shows count & opens Photos)
    fill_rounded_rect_gradient(
        fb,
        fb_w,
        fb_h,
        28,
        h - 104,
        48,
        48,
        8,
        rgb(0, 150, 199),
        rgb(244, 162, 97),
    );
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        36,
        h - 86,
        &format!("#{}", screen_state.photos_count.max(6)),
        rgb(255, 255, 255),
        1,
        2,
        true,
    );

    // Center Shutter Button
    let sh_col = if cur_mode == 0 {
        rgb(255, 59, 48)
    } else {
        rgb(255, 255, 255)
    };
    fill_circle(fb, fb_w, fb_h, w / 2, h - 80, 34, rgb(255, 255, 255));
    fill_circle(fb, fb_w, fb_h, w / 2, h - 80, 28, sh_col);
    draw_ring(fb, fb_w, fb_h, w / 2, h - 80, 28, 31, rgb(0, 0, 0));

    // Bottom-right Flip Camera Button
    fill_circle(fb, fb_w, fb_h, w - 52, h - 80, 22, rgba(255, 255, 255, 48));
    draw_ring(fb, fb_w, fb_h, w - 52, h - 80, 8, 10, rgb(255, 255, 255));

    draw_home_indicator(fb, fb_w, fb_h, screen_state, false);
}

/// Renders the interactive iOS 16 Photos app (3-column Library Grid + Full-Screen Photo Viewer).
fn render_photos_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    let h = fb_h as i32;
    fb.fill(rgb(255, 255, 255));
    draw_iphone_x_status_bar(fb, fb_w, fb_h, true, screen_state);

    let palettes: [(u32, u32); 6] = [
        (rgb(0, 150, 199), rgb(244, 162, 97)),
        (rgb(255, 94, 98), rgb(255, 153, 102)),
        (rgb(17, 153, 142), rgb(56, 239, 125)),
        (rgb(142, 45, 226), rgb(74, 0, 224)),
        (rgb(29, 155, 240), rgb(0, 97, 242)),
        (rgb(20, 30, 48), rgb(36, 59, 85)),
    ];

    if let Some(sel) = screen_state.photos_selected {
        draw_subpage_header(
            fb,
            fb_w,
            fb_h,
            "Library",
            &format!("Photo #{} — Oct 20", sel + 1),
            false,
        );
        let (c0, c1) = palettes[sel % palettes.len()];
        fill_rounded_rect_gradient(fb, fb_w, fb_h, 12, 92, w - 24, 580, 16, c0, c1);
        fill_circle(fb, fb_w, fb_h, w / 2 - 40, 220, 38, rgba(255, 220, 120, 180));
        fill_triangle(
            fb,
            fb_w,
            fb_h,
            12,
            660,
            140,
            420,
            280,
            660,
            rgba(18, 28, 48, 165),
        );
        fill_triangle(
            fb,
            fb_w,
            fb_h,
            110,
            660,
            245,
            450,
            w - 12,
            660,
            rgba(10, 18, 34, 190),
        );

        let is_fav = (screen_state.photos_favorites & (1 << (sel % 16))) != 0;
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            24,
            734,
            "Share",
            rgb(0, 122, 255),
            1,
            2,
            false,
        );
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            w / 2 - 34,
            734,
            if is_fav { "<3 Favorited" } else { "<3 Favorite" },
            if is_fav {
                rgb(255, 45, 85)
            } else {
                rgb(0, 122, 255)
            },
            1,
            2,
            false,
        );
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            w - 72,
            734,
            "Delete",
            rgb(255, 59, 48),
            1,
            2,
            false,
        );
    } else {
        let count = screen_state.photos_count.max(6);
        draw_text_semibold(fb, fb_w, fb_h, 16, 48, "Photos", rgb(18, 18, 22), 2, 2, false);
        draw_text_right(
            fb,
            fb_w,
            fb_h,
            w - 16,
            54,
            &format!("{} Photos • Tap any to view", count),
            rgb(142, 142, 147),
            1,
            2,
        );

        for idx in 0..count.min(15) {
            let col = (idx as i32) % 3;
            let row = (idx as i32) / 3;
            let tx = 14 + col * 112;
            let ty = 96 + row * 112;
            let (c0, c1) = palettes[idx % palettes.len()];
            fill_rounded_rect_gradient(fb, fb_w, fb_h, tx, ty, 106, 106, 8, c0, c1);
            fill_circle(fb, fb_w, fb_h, tx + 32, ty + 32, 14, rgba(255, 230, 120, 170));
            if (screen_state.photos_favorites & (1 << (idx % 16))) != 0 {
                draw_text_semibold(
                    fb,
                    fb_w,
                    fb_h,
                    tx + 8,
                    ty + 84,
                    "<3",
                    rgb(255, 255, 255),
                    1,
                    2,
                    true,
                );
            }
        }

        let tab_y = h - 56;
        fill_rect(fb, fb_w, fb_h, 0, tab_y, w, 1, rgb(218, 218, 224));
        let tabs = ["Library", "For You", "Albums", "Search"];
        for (i, &t) in tabs.iter().enumerate() {
            let col = if i as u8 == screen_state.photos_tab {
                rgb(0, 122, 255)
            } else {
                rgb(142, 142, 147)
            };
            draw_text_scaled(fb, fb_w, fb_h, 28 + (i as i32) * 86, tab_y + 18, t, col, 1, 2);
        }
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, true);
}

/// Renders the interactive iOS 16 Clock app (World Clock, Alarm, Stopwatch, Timer).
fn render_clock_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    let h = fb_h as i32;
    fb.fill(rgb(0, 0, 0));
    draw_iphone_x_status_bar(fb, fb_w, fb_h, false, screen_state);

    let tab = screen_state.clock_tab;
    let titles = ["World Clock", "Alarm", "Stopwatch", "Timer"];
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        20,
        48,
        titles[(tab as usize).min(3)],
        rgb(255, 255, 255),
        2,
        2,
        false,
    );

    match tab {
        0 => {
            let cities = [
                ("Today, +0HRS", "Ho Chi Minh City", "9:41"),
                ("Today, +0HRS", "Ha Noi", "9:41"),
                ("Yesterday, -14HRS", "Cupertino", "19:41"),
                ("Today, +2HRS", "Tokyo", "11:41"),
            ];
            for (i, &(off, city, tm)) in cities.iter().enumerate() {
                let ry = 96 + (i as i32) * 74;
                fill_rounded_rect(fb, fb_w, fb_h, 16, ry, w - 32, 64, 12, rgb(28, 28, 30));
                draw_text_scaled(fb, fb_w, fb_h, 28, ry + 14, off, rgb(142, 142, 147), 1, 2);
                draw_text_semibold(
                    fb,
                    fb_w,
                    fb_h,
                    28,
                    ry + 34,
                    city,
                    rgb(255, 255, 255),
                    1,
                    2,
                    false,
                );
                draw_text_right(fb, fb_w, fb_h, w - 28, ry + 22, tm, rgb(255, 255, 255), 2, 2);
            }
        }
        1 => {
            let alarms = [
                ("06:30", "Wake Up • Every Day"),
                ("07:00", "Morning Workout"),
                ("08:30", "Daily Standup — ios-sweet"),
                ("22:30", "Sleep / Wind Down"),
            ];
            for (i, &(tm, lbl)) in alarms.iter().enumerate() {
                let ry = 96 + (i as i32) * 62;
                let on = (screen_state.alarms_enabled & (1 << i)) != 0;
                fill_rounded_rect(fb, fb_w, fb_h, 16, ry, w - 32, 54, 12, rgb(28, 28, 30));
                let c = if on {
                    rgb(255, 255, 255)
                } else {
                    rgb(120, 120, 128)
                };
                draw_text_semibold(fb, fb_w, fb_h, 28, ry + 10, tm, c, 2, 2, false);
                draw_text_scaled(fb, fb_w, fb_h, 28, ry + 34, lbl, rgb(142, 142, 147), 1, 2);
                draw_ios_switch(fb, fb_w, fb_h, w - 68, ry + 16, on);
            }
        }
        2 => {
            let ticks = screen_state.stopwatch_elapsed_ticks;
            let mins = (ticks / 3600) % 60;
            let secs = (ticks / 60) % 60;
            let cs = ((ticks % 60) * 100) / 60;
            let sw_str = format!("{:02}:{:02}.{:02}", mins, secs, cs);
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                180,
                &sw_str,
                rgb(255, 255, 255),
                3,
                3,
                false,
            );

            // Lap/Reset (left) & Start/Stop (right) circles
            fill_circle(fb, fb_w, fb_h, 76, 350, 38, rgb(56, 56, 60));
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                58,
                344,
                if screen_state.stopwatch_running {
                    "Lap"
                } else {
                    "Reset"
                },
                rgb(255, 255, 255),
                1,
                2,
                false,
            );

            let (btn_bg, btn_lbl) = if screen_state.stopwatch_running {
                (rgb(180, 38, 34), "Stop")
            } else {
                (rgb(24, 118, 48), "Start")
            };
            fill_circle(fb, fb_w, fb_h, w - 76, 350, 38, btn_bg);
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                w - 92,
                344,
                btn_lbl,
                rgb(255, 255, 255),
                1,
                2,
                false,
            );

            for (i, &lap_t) in screen_state.stopwatch_laps.iter().enumerate() {
                let l_str = format!(
                    "Lap {}                                {:02}:{:02}.{:02}",
                    i + 1,
                    (lap_t / 3600) % 60,
                    (lap_t / 60) % 60,
                    ((lap_t % 60) * 100) / 60
                );
                draw_text_scaled(
                    fb,
                    fb_w,
                    fb_h,
                    32,
                    420 + (i as i32) * 28,
                    &l_str,
                    rgb(220, 220, 228),
                    1,
                    2,
                );
            }
        }
        _ => {
            let rem = screen_state.timer_remaining_ticks;
            let mins = (rem / 3600) % 60;
            let secs = (rem / 60) % 60;
            draw_ring(fb, fb_w, fb_h, w / 2, 235, 102, 110, rgb(255, 149, 0));
            draw_text_centered(
                fb,
                fb_w,
                fb_h,
                222,
                &format!("{:02}:{:02}", mins, secs),
                rgb(255, 255, 255),
                3,
                3,
                false,
            );

            fill_circle(fb, fb_w, fb_h, 76, 445, 38, rgb(56, 56, 60));
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                54,
                439,
                "Cancel",
                rgb(255, 255, 255),
                1,
                2,
                false,
            );

            let (bg, lbl) = if screen_state.timer_running {
                (rgb(204, 118, 0), "Pause")
            } else {
                (rgb(24, 118, 48), "Start")
            };
            fill_circle(fb, fb_w, fb_h, w - 76, 445, 38, bg);
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                w - 94,
                439,
                lbl,
                rgb(255, 255, 255),
                1,
                2,
                false,
            );
        }
    }

    let tab_y = h - 56;
    fill_rect(fb, fb_w, fb_h, 0, tab_y, w, 1, rgb(44, 44, 46));
    for (i, &t) in titles.iter().enumerate() {
        let col = if i as u8 == tab {
            rgb(255, 149, 0)
        } else {
            rgb(142, 142, 147)
        };
        draw_text_scaled(fb, fb_w, fb_h, 18 + (i as i32) * 88, tab_y + 18, t, col, 1, 2);
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, false);
}

/// Renders the interactive iOS 16 Notes app (Notes list + Note Editor with Keyboard).
fn render_notes_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    fb.fill(rgb(242, 242, 247));
    draw_iphone_x_status_bar(fb, fb_w, fb_h, true, screen_state);

    if let Some(idx) = screen_state.notes_selected {
        let (title, body) = screen_state
            .notes_items
            .get(idx)
            .cloned()
            .unwrap_or_else(|| ("Note".to_string(), String::new()));
        draw_subpage_header(fb, fb_w, fb_h, "Notes", &title, false);
        fill_rounded_rect(fb, fb_w, fb_h, 16, 84, w - 32, 410, 12, rgb(255, 255, 255));
        draw_text_semibold(fb, fb_w, fb_h, 28, 102, &title, rgb(18, 18, 22), 2, 2, false);
        for (li, line) in body.lines().take(12).enumerate() {
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                28,
                140 + (li as i32) * 24,
                line,
                rgb(48, 50, 58),
                1,
                2,
            );
        }
        draw_ios_keyboard(fb, fb_w, fb_h, false, "Return");
    } else {
        draw_text_semibold(fb, fb_w, fb_h, 20, 48, "Notes", rgb(18, 18, 22), 2, 2, false);
        draw_text_right(
            fb,
            fb_w,
            fb_h,
            w - 20,
            52,
            "+ New Note",
            rgb(228, 160, 0),
            1,
            2,
        );

        let default_notes = [
            (
                "iOS 16.7.16 Bring-Up Plan",
                "1. Boot XNU 22.6.0 on sweet-virt",
            ),
            ("Shopping & Devices", "- iPhone X (iPhone10,3 / D22AP)"),
            ("Quick Scratchpad", "Tap any note to edit with keyboard!"),
        ];
        let count = screen_state.notes_items.len().max(3);
        fill_rounded_rect(
            fb,
            fb_w,
            fb_h,
            16,
            104,
            w - 32,
            (count as i32) * 62,
            12,
            rgb(255, 255, 255),
        );
        for i in 0..count {
            let (t, b) = screen_state
                .notes_items
                .get(i)
                .map(|(a, c)| (a.as_str(), c.lines().next().unwrap_or("")))
                .unwrap_or(default_notes[i % default_notes.len()]);
            let ry = 104 + (i as i32) * 62;
            if i > 0 {
                fill_rect(fb, fb_w, fb_h, 28, ry, w - 44, 1, rgb(230, 230, 235));
            }
            draw_text_semibold(fb, fb_w, fb_h, 28, ry + 12, t, rgb(18, 18, 22), 1, 2, false);
            draw_text_scaled(fb, fb_w, fb_h, 28, ry + 34, b, rgb(138, 138, 144), 1, 2);
        }
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, true);
}

/// Renders the interactive iOS 16 Reminders app (toggleable checklist).
fn render_reminders_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    fb.fill(rgb(242, 242, 247));
    draw_iphone_x_status_bar(fb, fb_w, fb_h, true, screen_state);

    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        20,
        48,
        "Reminders",
        rgb(0, 122, 255),
        2,
        2,
        false,
    );
    draw_text_right(fb, fb_w, fb_h, w - 20, 52, "+ Add", rgb(0, 122, 255), 1, 2);
    draw_text_scaled(
        fb,
        fb_w,
        fb_h,
        20,
        82,
        "Tap any task row below to check / uncheck:",
        rgb(138, 138, 144),
        1,
        2,
    );

    let default_tasks = [
        "Test Settings Wi-Fi, Dark Mode & About",
        "Dial a number in Phone & test Call UI",
        "Calculate expressions in Calculator",
        "Browse websites in Safari & send iMessage",
        "Take photos in Camera & view in Photos",
    ];
    let len = screen_state.reminders_items.len().max(5);
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        16,
        112,
        w - 32,
        (len as i32) * 50,
        12,
        rgb(255, 255, 255),
    );
    for i in 0..len {
        let task = screen_state
            .reminders_items
            .get(i)
            .map(|s| s.as_str())
            .unwrap_or(default_tasks[i % default_tasks.len()]);
        let done = (screen_state.reminders_done & (1 << i)) != 0;
        let ry = 112 + (i as i32) * 50;
        if i > 0 {
            fill_rect(fb, fb_w, fb_h, 52, ry, w - 68, 1, rgb(230, 230, 235));
        }
        if done {
            fill_circle(fb, fb_w, fb_h, 34, ry + 25, 10, rgb(0, 122, 255));
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                30,
                ry + 19,
                "v",
                rgb(255, 255, 255),
                1,
                2,
                false,
            );
        } else {
            draw_ring(fb, fb_w, fb_h, 34, ry + 25, 8, 10, rgb(192, 192, 198));
        }
        draw_text_scaled(
            fb,
            fb_w,
            fb_h,
            54,
            ry + 19,
            task,
            if done {
                rgb(152, 152, 158)
            } else {
                rgb(18, 18, 22)
            },
            1,
            2,
        );
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, true);
}

/// Renders the interactive Apple Music player app.
fn render_music_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    let (title, artist, album, c0, c1) =
        MUSIC_TRACKS[screen_state.music_track_idx % MUSIC_TRACKS.len()];

    fill_rounded_rect_gradient(fb, fb_w, fb_h, 0, 0, w, fb_h as i32, 0, c1, rgb(12, 14, 22));
    draw_iphone_x_status_bar(fb, fb_w, fb_h, false, screen_state);

    // Album Artwork Squircle Card
    fill_rounded_rect_gradient(fb, fb_w, fb_h, 44, 78, w - 88, w - 88, 22, c0, c1);
    fill_circle(fb, fb_w, fb_h, w / 2, 78 + (w - 88) / 2, 46, rgba(255, 255, 255, 42));
    draw_text_centered(
        fb,
        fb_w,
        fb_h,
        78 + (w - 88) / 2 - 8,
        "Apple Music",
        rgb(255, 255, 255),
        2,
        2,
        true,
    );

    draw_text_semibold(fb, fb_w, fb_h, 36, 386, title, rgb(255, 255, 255), 2, 2, false);
    draw_text_scaled(fb, fb_w, fb_h, 36, 414, artist, rgba(255, 255, 255, 210), 1, 2);
    draw_text_scaled(fb, fb_w, fb_h, 36, 434, album, rgba(255, 255, 255, 150), 1, 2);

    // Progress Bar
    let prog_x = 36;
    let prog_w = w - 72;
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        prog_x,
        476,
        prog_w,
        5,
        2,
        rgba(255, 255, 255, 64),
    );
    let fill_p = (((screen_state.music_progress_ticks % 3600) as i32 * prog_w) / 3600).max(24);
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        prog_x,
        476,
        fill_p,
        5,
        2,
        rgb(255, 255, 255),
    );

    // Playback Buttons: <<  Play/Pause  >>
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        78,
        548,
        "<<",
        rgb(255, 255, 255),
        2,
        2,
        false,
    );
    fill_circle(fb, fb_w, fb_h, w / 2, 556, 32, rgba(255, 255, 255, 42));
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        w / 2 - 18,
        548,
        if screen_state.music_playing {
            "PAUSE"
        } else {
            "PLAY"
        },
        rgb(255, 255, 255),
        1,
        2,
        false,
    );
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        w - 102,
        548,
        ">>",
        rgb(255, 255, 255),
        2,
        2,
        false,
    );

    // Volume Bar
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        48,
        642,
        w - 96,
        6,
        3,
        rgba(255, 255, 255, 64),
    );
    let vol_w = ((w - 96) * (screen_state.volume.max(50) as i32)) / 100;
    fill_rounded_rect(fb, fb_w, fb_h, 48, 642, vol_w, 6, 3, rgb(255, 255, 255));

    draw_home_indicator(fb, fb_w, fb_h, screen_state, false);
}

/// Renders the interactive iOS 16 Weather app (tap anywhere to cycle Ho Chi Minh City / Ha Noi / Cupertino).
fn render_weather_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    let (city, temp, cond, hi, lo, summary) =
        WEATHER_CITIES[screen_state.weather_city_idx % WEATHER_CITIES.len()];

    fill_rounded_rect_gradient(
        fb,
        fb_w,
        fb_h,
        0,
        0,
        w,
        fb_h as i32,
        0,
        rgb(24, 98, 198),
        rgb(64, 156, 255),
    );
    draw_iphone_x_status_bar(fb, fb_w, fb_h, false, screen_state);

    draw_text_centered(fb, fb_w, fb_h, 64, city, rgb(255, 255, 255), 2, 2, true);
    draw_text_centered(
        fb,
        fb_w,
        fb_h,
        104,
        &format!("{}°C", temp),
        rgb(255, 255, 255),
        3,
        3,
        true,
    );
    draw_text_centered(fb, fb_w, fb_h, 152, cond, rgba(255, 255, 255, 230), 1, 2, true);
    draw_text_centered(
        fb,
        fb_w,
        fb_h,
        172,
        &format!("H:{}°  L:{}°   (Tap to switch city)", hi, lo),
        rgb(255, 255, 255),
        1,
        2,
        true,
    );

    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        16,
        212,
        w - 32,
        104,
        16,
        rgba(18, 56, 122, 130),
    );
    draw_text_scaled(fb, fb_w, fb_h, 28, 226, summary, rgb(255, 255, 255), 1, 2);
    let hrs = ["Now", "10AM", "11AM", "12PM", "1PM", "2PM"];
    for (i, &hr) in hrs.iter().enumerate() {
        let hx = 30 + (i as i32) * 52;
        draw_text_scaled(fb, fb_w, fb_h, hx, 258, hr, rgb(225, 240, 255), 1, 2);
        fill_circle(fb, fb_w, fb_h, hx + 10, 282, 6, rgb(255, 214, 10));
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            hx,
            294,
            &format!("{}°", temp + (i as i32 % 3) - 1),
            rgb(255, 255, 255),
            1,
            2,
            false,
        );
    }

    // 5-Day Forecast Card
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        16,
        332,
        w - 32,
        210,
        16,
        rgba(18, 56, 122, 130),
    );
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        28,
        346,
        "10-DAY FORECAST",
        rgba(220, 235, 255, 200),
        1,
        2,
        false,
    );
    let days = [
        ("Today", lo, hi),
        ("Wed", lo - 1, hi + 1),
        ("Thu", lo, hi),
        ("Fri", lo + 1, hi + 2),
        ("Sat", lo, hi + 1),
    ];
    for (i, &(d, l, h_t)) in days.iter().enumerate() {
        let ry = 374 + (i as i32) * 32;
        draw_text_semibold(fb, fb_w, fb_h, 28, ry, d, rgb(255, 255, 255), 1, 2, false);
        fill_circle(fb, fb_w, fb_h, 120, ry + 6, 6, rgb(255, 214, 10));
        draw_text_scaled(
            fb,
            fb_w,
            fb_h,
            160,
            ry,
            &format!("{}°", l),
            rgba(220, 235, 255, 190),
            1,
            2,
        );
        fill_rounded_rect(fb, fb_w, fb_h, 196, ry + 4, 86, 6, 3, rgb(255, 180, 0));
        draw_text_right(
            fb,
            fb_w,
            fb_h,
            w - 28,
            ry,
            &format!("{}°", h_t),
            rgb(255, 255, 255),
            1,
            2,
        );
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, false);
}

/// Renders the interactive iOS 16 Files app (browses the real iOS 16.7.16 rootfs tree).
fn render_files_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    fb.fill(rgb(242, 242, 247));
    draw_iphone_x_status_bar(fb, fb_w, fb_h, true, screen_state);

    if screen_state.files_current_dir.is_empty() {
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            20,
            48,
            "On My iPhone X",
            rgb(18, 18, 22),
            2,
            2,
            false,
        );
        draw_text_scaled(
            fb,
            fb_w,
            fb_h,
            20,
            78,
            "ios16-root-20H392.img (APFS / Darwin 22.6.0)",
            rgb(138, 138, 144),
            1,
            2,
        );
        let folders = [
            ("Applications", "48 System App Bundles (.app)"),
            ("System", "/System/Library/Frameworks & CoreServices"),
            ("Library", "Caches, Preferences, MobileSubstrate"),
            ("sbin", "launchd, mount_apfs, fsck_apfs"),
            ("private", "/private/var/mobile & containers"),
            ("Developer", "Xcode DDI & Metal Debug Symbols"),
            ("usr", "/usr/lib/dyld, libSystem.B.dylib"),
        ];
        fill_rounded_rect(
            fb,
            fb_w,
            fb_h,
            16,
            110,
            w - 32,
            folders.len() as i32 * 52,
            12,
            rgb(255, 255, 255),
        );
        for (i, &(f, desc)) in folders.iter().enumerate() {
            let ry = 110 + (i as i32) * 52;
            if i > 0 {
                fill_rect(fb, fb_w, fb_h, 64, ry, w - 80, 1, rgb(230, 230, 235));
            }
            fill_rounded_rect(fb, fb_w, fb_h, 28, ry + 14, 26, 20, 4, rgb(0, 122, 255));
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                64,
                ry + 10,
                &format!("/{}", f),
                rgb(18, 18, 22),
                1,
                2,
                false,
            );
            draw_text_scaled(fb, fb_w, fb_h, 64, ry + 28, desc, rgb(138, 138, 144), 1, 2);
            draw_text_right(fb, fb_w, fb_h, w - 28, ry + 18, ">", rgb(192, 192, 198), 1, 2);
        }
    } else {
        let dir = &screen_state.files_current_dir;
        draw_subpage_header(fb, fb_w, fb_h, "Root", &format!("/{}", dir), false);
        let mut entries = Vec::new();
        let host_path = PathBuf::from("research/ipsw-x/os_decompressed").join(dir);
        if let Ok(rd) = fs::read_dir(&host_path) {
            for e in rd.flatten().take(9) {
                if let Some(nm) = e.file_name().to_str() {
                    entries.push(nm.to_string());
                }
            }
        }
        if entries.is_empty() {
            entries = match dir.as_str() {
                "Applications" => vec![
                    "Preferences.app".to_string(),
                    "MobileSafari.app".to_string(),
                    "MobilePhone.app".to_string(),
                    "MobileSMS.app".to_string(),
                    "Camera.app".to_string(),
                    "MobileSlideShow.app".to_string(),
                    "Calculator.app".to_string(),
                    "MobileTimer.app".to_string(),
                ],
                "sbin" => vec![
                    "launchd".to_string(),
                    "mount_apfs".to_string(),
                    "fsck_apfs".to_string(),
                    "pfctl".to_string(),
                ],
                _ => vec![
                    "Library".to_string(),
                    "Frameworks".to_string(),
                    "CoreServices".to_string(),
                    "SpringBoard.app".to_string(),
                    "dyld_shared_cache_arm64".to_string(),
                ],
            };
        }
        fill_rounded_rect(
            fb,
            fb_w,
            fb_h,
            16,
            92,
            w - 32,
            (entries.len() as i32) * 44,
            12,
            rgb(255, 255, 255),
        );
        for (i, nm) in entries.iter().enumerate() {
            let ry = 92 + (i as i32) * 44;
            if i > 0 {
                fill_rect(fb, fb_w, fb_h, 28, ry, w - 44, 1, rgb(230, 230, 235));
            }
            draw_text_semibold(fb, fb_w, fb_h, 28, ry + 15, nm, rgb(18, 18, 22), 1, 2, false);
            draw_text_right(
                fb,
                fb_w,
                fb_h,
                w - 28,
                ry + 15,
                "APFS",
                rgb(142, 142, 147),
                1,
                2,
            );
        }
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, true);
}

/// Renders the interactive iOS 16 App Store app.
fn render_app_store_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    let h = fb_h as i32;
    fb.fill(rgb(255, 255, 255));
    draw_iphone_x_status_bar(fb, fb_w, fb_h, true, screen_state);

    let tabs = ["Today", "Games", "Apps", "Search"];
    let cur_tab = (screen_state.app_store_tab as usize).min(3);
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        20,
        48,
        tabs[cur_tab],
        rgb(18, 18, 22),
        2,
        2,
        false,
    );

    // Featured Hero Banner
    fill_rounded_rect_gradient(
        fb,
        fb_w,
        fb_h,
        20,
        84,
        w - 40,
        148,
        16,
        rgb(20, 32, 78),
        rgb(0, 122, 255),
    );
    draw_text_scaled(
        fb,
        fb_w,
        fb_h,
        34,
        102,
        "FEATURED ON IPHONE X",
        rgb(160, 210, 255),
        1,
        2,
    );
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        34,
        124,
        "Sweet 3D Metal Engine",
        rgb(255, 255, 255),
        2,
        2,
        false,
    );
    draw_text_scaled(
        fb,
        fb_w,
        fb_h,
        34,
        156,
        "60 FPS Hardware-Accelerated Triangle Ring",
        rgb(225, 238, 255),
        1,
        2,
    );

    let rows = [
        ("Sweet 3D Bench", "Real-Time 3D Cube & Starfield", "OPEN"),
        ("Calculator", "Apple Scientific & Standard Math", "OPEN"),
        ("Safari", "WebKit Browser for iOS 16.7.16", "OPEN"),
        ("Music", "Lossless & Dolby Atmos Audio", "OPEN"),
    ];
    for (i, &(name, sub, btn)) in rows.iter().enumerate() {
        let ry = 252 + (i as i32) * 68;
        render_squircle_icon(fb, fb_w, fb_h, 20, ry + 6, 48, false, |canvas, cw, _ch| {
            draw_stock_icon_canvas(canvas, cw, name);
        });
        draw_text_semibold(fb, fb_w, fb_h, 80, ry + 12, name, rgb(18, 18, 22), 1, 2, false);
        draw_text_scaled(fb, fb_w, fb_h, 80, ry + 32, sub, rgb(138, 138, 144), 1, 2);
        fill_rounded_rect(fb, fb_w, fb_h, w - 84, ry + 16, 64, 28, 14, rgb(236, 238, 246));
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            w - 68,
            ry + 24,
            btn,
            rgb(0, 122, 255),
            1,
            2,
            false,
        );
    }

    let tab_y = h - 56;
    fill_rect(fb, fb_w, fb_h, 0, tab_y, w, 1, rgb(218, 218, 224));
    for (i, &t) in tabs.iter().enumerate() {
        let col = if i == cur_tab {
            rgb(0, 122, 255)
        } else {
            rgb(142, 142, 147)
        };
        draw_text_scaled(fb, fb_w, fb_h, 28 + (i as i32) * 88, tab_y + 18, t, col, 1, 2);
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, true);
}

/// Renders the interactive iOS 16 Control Center overlay.
fn render_control_center_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    draw_ios16_wallpaper(
        fb,
        fb_w,
        fb_h,
        screen_state.dark_mode,
        screen_state.frame_tick,
    );
    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        0,
        0,
        w,
        fb_h as i32,
        0,
        14,
        rgba(18, 24, 42, 140),
        0,
    );
    draw_iphone_x_status_bar(fb, fb_w, fb_h, false, screen_state);

    let margin = 24;
    let gap = 14;
    let mod_size = (w - 2 * margin - gap) / 2;
    let top_y = 64;

    // Top-Left Connectivity Module
    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        margin,
        top_y,
        mod_size,
        mod_size,
        22,
        8,
        rgba(30, 36, 54, 155),
        rgba(255, 255, 255, 32),
    );
    fill_circle(
        fb,
        fb_w,
        fb_h,
        margin + 40,
        top_y + 40,
        22,
        if screen_state.airplane_mode {
            rgb(255, 149, 0)
        } else {
            rgba(255, 255, 255, 45)
        },
    );
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        margin + 30,
        top_y + 34,
        "AIR",
        rgb(255, 255, 255),
        1,
        2,
        false,
    );

    fill_circle(
        fb,
        fb_w,
        fb_h,
        margin + mod_size - 40,
        top_y + 40,
        22,
        if screen_state.cellular_enabled && !screen_state.airplane_mode {
            rgb(52, 199, 89)
        } else {
            rgba(255, 255, 255, 45)
        },
    );
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        margin + mod_size - 48,
        top_y + 34,
        "5G",
        rgb(255, 255, 255),
        1,
        2,
        false,
    );

    fill_circle(
        fb,
        fb_w,
        fb_h,
        margin + 40,
        top_y + mod_size - 40,
        22,
        if screen_state.wifi_enabled {
            rgb(0, 122, 255)
        } else {
            rgba(255, 255, 255, 45)
        },
    );
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        margin + 26,
        top_y + mod_size - 46,
        "WiFi",
        rgb(255, 255, 255),
        1,
        2,
        false,
    );

    fill_circle(
        fb,
        fb_w,
        fb_h,
        margin + mod_size - 40,
        top_y + mod_size - 40,
        22,
        if screen_state.bluetooth_enabled {
            rgb(0, 122, 255)
        } else {
            rgba(255, 255, 255, 45)
        },
    );
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        margin + mod_size - 48,
        top_y + mod_size - 46,
        "BT",
        rgb(255, 255, 255),
        1,
        2,
        false,
    );

    // Top-Right Now Playing Module
    let right_x = margin + mod_size + gap;
    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        right_x,
        top_y,
        mod_size,
        mod_size,
        22,
        8,
        rgba(30, 36, 54, 155),
        rgba(255, 255, 255, 32),
    );
    let track_name = if screen_state.music_playing {
        MUSIC_TRACKS[screen_state.music_track_idx % MUSIC_TRACKS.len()].0
    } else {
        "Not Playing"
    };
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        right_x + 18,
        top_y + 42,
        track_name,
        rgb(255, 255, 255),
        1,
        2,
        false,
    );
    fill_triangle(
        fb,
        fb_w,
        fb_h,
        right_x + mod_size / 2 - 6,
        top_y + 92,
        right_x + mod_size / 2 - 6,
        top_y + 112,
        right_x + mod_size / 2 + 10,
        top_y + 102,
        rgb(255, 255, 255),
    );

    // Middle Row: Focus + Dark Mode on Left, Brightness & Volume Sliders on Right
    let mid_y = top_y + mod_size + gap;
    let tile_h = (mod_size - gap) / 2;
    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        margin,
        mid_y,
        mod_size,
        tile_h,
        18,
        8,
        if screen_state.focus_dnd {
            rgba(88, 86, 214, 210)
        } else {
            rgba(30, 36, 54, 155)
        },
        rgba(255, 255, 255, 32),
    );
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        margin + 24,
        mid_y + tile_h / 2 - 5,
        if screen_state.focus_dnd {
            "Focus: DND ON"
        } else {
            "Focus"
        },
        rgb(255, 255, 255),
        1,
        2,
        false,
    );

    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        margin,
        mid_y + tile_h + gap,
        mod_size,
        tile_h,
        18,
        8,
        if screen_state.dark_mode {
            rgba(255, 255, 255, 220)
        } else {
            rgba(30, 36, 54, 155)
        },
        rgba(255, 255, 255, 32),
    );
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        margin + 20,
        mid_y + tile_h + gap + tile_h / 2 - 5,
        if screen_state.dark_mode {
            "Dark Mode: ON"
        } else {
            "Dark Mode: OFF"
        },
        if screen_state.dark_mode {
            rgb(18, 18, 22)
        } else {
            rgb(255, 255, 255)
        },
        1,
        2,
        false,
    );

    let slider_w = (mod_size - gap) / 2;
    let slider_h = mod_size;
    let br_x = right_x;
    let vol_x = right_x + slider_w + gap;

    let br_pct = if screen_state.initialized {
        screen_state.brightness as i32
    } else {
        85
    };
    let vol_pct = if screen_state.initialized {
        screen_state.volume as i32
    } else {
        70
    };

    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        br_x,
        mid_y,
        slider_w,
        slider_h,
        18,
        8,
        rgba(30, 36, 54, 155),
        rgba(255, 255, 255, 32),
    );
    let br_fill = (slider_h * br_pct) / 100;
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        br_x,
        mid_y + slider_h - br_fill,
        slider_w,
        br_fill,
        18,
        rgba(255, 255, 255, 235),
    );

    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        vol_x,
        mid_y,
        slider_w,
        slider_h,
        18,
        8,
        rgba(30, 36, 54, 155),
        rgba(255, 255, 255, 32),
    );
    let vol_fill = (slider_h * vol_pct) / 100;
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        vol_x,
        mid_y + slider_h - vol_fill,
        slider_w,
        vol_fill,
        18,
        rgba(255, 255, 255, 235),
    );

    // Bottom 4 quick-toggle squircles (Flashlight, Timer, Calculator, Camera)
    let bot_y = mid_y + mod_size + gap;
    let q_size = (w - 2 * margin - 3 * gap) / 4;
    let q_labels = ["Torch", "Timer", "Calc", "Cam"];
    for (q_idx, &ql) in q_labels.iter().enumerate() {
        let qx = margin + (q_idx as i32) * (q_size + gap);
        let active = q_idx == 0 && screen_state.flashlight_on;
        blur_rounded_rect(
            fb,
            fb_w,
            fb_h,
            qx,
            bot_y,
            q_size,
            q_size,
            18,
            8,
            if active {
                rgba(255, 255, 255, 230)
            } else {
                rgba(30, 36, 54, 155)
            },
            rgba(255, 255, 255, 32),
        );
        let lw = measure_text_px(ql, 1);
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            qx + q_size / 2 - lw / 2,
            bot_y + q_size / 2 - 5,
            ql,
            if active {
                rgb(0, 122, 255)
            } else {
                rgb(255, 255, 255)
            },
            1,
            2,
            false,
        );
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, false);
}

/// Renders dedicated interactive views for Calendar, Mail, Maps, Health, Wallet, Home, Stocks, Books, Podcasts, TV, and FaceTime.
fn render_specialized_stock_app(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
    app_name: &str,
) {
    let w = fb_w as i32;
    fb.fill(rgb(242, 242, 247));
    draw_iphone_x_status_bar(fb, fb_w, fb_h, true, screen_state);
    draw_text_semibold(fb, fb_w, fb_h, 20, 48, app_name, rgb(18, 18, 22), 2, 2, false);

    if app_name.eq_ignore_ascii_case("Calendar") {
        draw_text_right(
            fb,
            fb_w,
            fb_h,
            w - 20,
            52,
            "October 2026",
            rgb(255, 59, 48),
            1,
            2,
        );
        fill_rounded_rect(fb, fb_w, fb_h, 16, 86, w - 32, 240, 14, rgb(255, 255, 255));
        let sel_day = screen_state.calendar_selected_day.max(1);
        for d in 1..=31u8 {
            let idx = (d - 1) as i32;
            let col = idx % 7;
            let row = idx / 7;
            let cx = 40 + col * 46;
            let cy = 114 + row * 40;
            if d == sel_day {
                fill_circle(fb, fb_w, fb_h, cx, cy, 15, rgb(255, 59, 48));
            }
            let s = format!("{}", d);
            let sw = measure_text_px(&s, 1);
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                cx - sw / 2,
                cy - 6,
                &s,
                if d == sel_day {
                    rgb(255, 255, 255)
                } else {
                    rgb(18, 18, 22)
                },
                1,
                2,
                false,
            );
        }
        fill_rounded_rect(fb, fb_w, fb_h, 16, 342, w - 32, 92, 14, rgb(255, 255, 255));
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            30,
            358,
            &format!("Oct {} • iOS 16.7.16 Milestone Review", sel_day),
            rgb(18, 18, 22),
            1,
            2,
            false,
        );
        draw_text_scaled(
            fb,
            fb_w,
            fb_h,
            30,
            384,
            "10:00 AM — All interactive iOS apps verified!",
            rgb(120, 122, 130),
            1,
            2,
        );
    } else if app_name.eq_ignore_ascii_case("Home") {
        draw_text_scaled(
            fb,
            fb_w,
            fb_h,
            20,
            78,
            "My Smart Home — Tap any tile to toggle ON/OFF",
            rgb(120, 122, 130),
            1,
            2,
        );
        let devs = [
            "Living Room Lights",
            "Bedroom AC (24°C)",
            "HomePod mini",
            "Smart Front Lock",
        ];
        for (i, &d) in devs.iter().enumerate() {
            let col = (i as i32) % 2;
            let row = (i as i32) / 2;
            let tx = 18 + col * 166;
            let ty = 115 + row * 106;
            let on = (screen_state.home_devices_on & (1 << i)) != 0;
            fill_rounded_rect(
                fb,
                fb_w,
                fb_h,
                tx,
                ty,
                156,
                92,
                16,
                if on {
                    rgb(255, 255, 255)
                } else {
                    rgb(220, 222, 230)
                },
            );
            draw_text_semibold(fb, fb_w, fb_h, tx + 14, ty + 20, d, rgb(18, 18, 22), 1, 2, false);
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                tx + 14,
                ty + 54,
                if on { "ON • Active" } else { "OFF" },
                if on {
                    rgb(255, 149, 0)
                } else {
                    rgb(142, 142, 147)
                },
                1,
                2,
                false,
            );
        }
    } else if app_name.eq_ignore_ascii_case("Mail") {
        if let Some(m_idx) = screen_state.mail_selected_idx {
            draw_subpage_header(fb, fb_w, fb_h, "Inbox", "Message", false);
            fill_rounded_rect(fb, fb_w, fb_h, 16, 86, w - 32, 260, 14, rgb(255, 255, 255));
            draw_text_semibold(
                fb,
                fb_w,
                fb_h,
                28,
                104,
                &format!("Apple Developer Update #{}", m_idx + 1),
                rgb(18, 18, 22),
                1,
                2,
                false,
            );
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                28,
                136,
                "Your iPhone X (iOS 16.7.16 / 20H392) is registered",
                rgb(60, 62, 70),
                1,
                2,
            );
            draw_text_scaled(
                fb,
                fb_w,
                fb_h,
                28,
                160,
                "with full SpringBoard & app interactivity.",
                rgb(60, 62, 70),
                1,
                2,
            );
        } else {
            let mails = [
                ("Apple", "Welcome to iPhone X on iOS 16.7.16"),
                ("iCloud+", "Your 50 GB iCloud storage is active"),
                ("App Store", "Sweet 3D Metal Engine is ready to test"),
                ("GitHub", "[ios-sweet] Full interactive apps merged"),
            ];
            fill_rounded_rect(fb, fb_w, fb_h, 16, 96, w - 32, 272, 14, rgb(255, 255, 255));
            for (i, &(from, subj)) in mails.iter().enumerate() {
                let ry = 96 + (i as i32) * 68;
                if i > 0 {
                    fill_rect(fb, fb_w, fb_h, 28, ry, w - 44, 1, rgb(230, 230, 235));
                }
                fill_circle(fb, fb_w, fb_h, 24, ry + 22, 4, rgb(0, 122, 255));
                draw_text_semibold(fb, fb_w, fb_h, 36, ry + 14, from, rgb(18, 18, 22), 1, 2, false);
                draw_text_scaled(fb, fb_w, fb_h, 36, ry + 36, subj, rgb(120, 122, 130), 1, 2);
            }
        }
    } else if app_name.eq_ignore_ascii_case("Maps") {
        fill_rounded_rect(fb, fb_w, fb_h, 16, 86, w - 32, 420, 16, rgb(235, 230, 218));
        fill_rect(fb, fb_w, fb_h, 16, 220, w - 32, 90, rgb(90, 200, 250));
        draw_line(fb, fb_w, fb_h, 24, 480, w - 24, 110, 8, rgb(255, 195, 20));
        fill_circle(fb, fb_w, fb_h, w / 2, 260, 10, rgb(0, 122, 255));
        draw_text_semibold(
            fb,
            fb_w,
            fb_h,
            w / 2 - 64,
            280,
            "Ho Chi Minh City • 5G",
            rgb(18, 18, 22),
            1,
            2,
            false,
        );
    } else {
        // Stocks, Health, Wallet, Books, Podcasts, TV, FaceTime
        let cards = [
            (
                format!("{} — Live Overview", app_name),
                "Connected to iCloud • iOS 16.7.16 (20H392)",
            ),
            (
                "Interactive Status: Online".to_string(),
                "Tap any item or swipe bottom bar to return Home",
            ),
            (
                "Hardware Acceleration".to_string(),
                "Apple A11 Bionic / D22AP Profile Active",
            ),
        ];
        for (i, (t, s)) in cards.iter().enumerate() {
            let ry = 96 + (i as i32) * 108;
            fill_rounded_rect(fb, fb_w, fb_h, 16, ry, w - 32, 94, 14, rgb(255, 255, 255));
            render_squircle_icon(fb, fb_w, fb_h, 30, ry + 20, 48, false, |canvas, cw, _ch| {
                draw_stock_icon_canvas(canvas, cw, app_name);
            });
            draw_text_semibold(fb, fb_w, fb_h, 92, ry + 24, t, rgb(18, 18, 22), 1, 2, false);
            draw_text_scaled(fb, fb_w, fb_h, 92, ry + 48, s, rgb(132, 134, 142), 1, 2);
        }
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, true);
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::SweetGpuEngine;

    #[test]
    fn test_interactive_ios16_os_and_all_apps() {
        let mut engine = SweetGpuEngine::new();
        let mut fb = vec![0u32; 360 * 800];
        let mut state = IosSimState::default();
        state.ensure_initialized();
        state.locked = true;

        // 1. Lock Screen: toggle flashlight, then tap bottom bar to unlock
        let msg = state.handle_tap(61, 711);
        assert!(msg.is_some());
        assert!(state.flashlight_on);
        let unlock_msg = state.handle_tap(180, 770);
        assert!(unlock_msg.is_some());
        assert!(!state.locked);

        // 2. Settings: test subpages, switches, Dark Mode, Low Power Mode, About page
        state.active_app = Some("Settings".to_string());
        state.settings_page.clear();
        engine.render_ios_springboard_frame(360, 800, &mut fb, &state);

        // Tap Airplane mode toggle at row 0 of Group 1 (y ~ 172..208, x = 310)
        state.handle_tap(310, 190);
        assert!(state.airplane_mode);
        state.handle_tap(310, 190);
        assert!(!state.airplane_mode);

        // Navigate Main -> General -> About -> Back -> Back
        state.settings_page = "General".to_string();
        engine.render_ios_springboard_frame(360, 800, &mut fb, &state);
        state.handle_tap(180, 100); // Row 0 in General is About (y: 86..122)
        assert_eq!(state.settings_page, "About");
        engine.render_ios_springboard_frame(360, 800, &mut fb, &state);
        state.handle_back_gesture();
        assert_eq!(state.settings_page, "General");
        state.handle_back_gesture();
        assert!(state.settings_page.is_empty());

        // Render all Settings subpages
        for subpage in [
            "AppleID",
            "WiFi",
            "Bluetooth",
            "Cellular",
            "General",
            "About",
            "SoftwareUpdate",
            "Storage",
            "Display",
            "Wallpaper",
            "Battery",
            "Sounds",
            "Focus",
            "Notifications",
            "ScreenTime",
            "ControlCenter",
        ] {
            state.settings_page = subpage.to_string();
            engine.render_ios_springboard_frame(360, 800, &mut fb, &state);
        }

        // 3. Calculator: 9 * 8 = 72
        state.active_app = Some("Calculator".to_string());
        state.calc_press("AC");
        state.calc_press("9");
        state.calc_press("×");
        state.calc_press("8");
        state.calc_press("=");
        assert_eq!(state.calc_display, "72");
        engine.render_ios_springboard_frame(360, 800, &mut fb, &state);

        // 4. Phone: dial and place call, tick timer, end call
        state.active_app = Some("Phone".to_string());
        state.phone_tab = 3;
        state.phone_dial_buf = "0901234567".to_string();
        state.start_phone_call(None);
        assert!(state.phone_in_call);
        for _ in 0..35 {
            state.tick();
        }
        assert!(state.frame_tick > state.phone_call_start_tick);
        engine.render_ios_springboard_frame(360, 800, &mut fb, &state);
        state.phone_in_call = false;

        // 5. Messages: open chat thread, type & send message, verify auto-reply
        state.active_app = Some("Messages".to_string());
        state.messages_active_chat = Some(0);
        let initial_len = state.messages_threads[0].2.len();
        for ch in "Hello iOS 16!".chars() {
            state.handle_key(ch);
        }
        state.handle_key('\n');
        assert!(state.messages_threads[0].2.len() >= initial_len + 2);
        engine.render_ios_springboard_frame(360, 800, &mut fb, &state);

        // 6. Camera & Photos: take photo shutter and verify photo count increments
        state.active_app = Some("Camera".to_string());
        let initial_photos = state.photos_count;
        state.handle_tap(180, 688); // Shutter button
        assert_eq!(state.photos_count, initial_photos + 1);
        engine.render_ios_springboard_frame(360, 800, &mut fb, &state);

        // 7. Render all 24 Home/Dock stock apps + Control Center + Spotlight
        state.spotlight_open = true;
        state.active_app = None;
        engine.render_ios_springboard_frame(360, 800, &mut fb, &state);
        state.spotlight_open = false;

        for app in HOME_GRID_APPS
            .iter()
            .chain(DOCK_APPS.iter())
            .chain(["ControlCenter"].iter())
        {
            state.active_app = Some((*app).to_string());
            engine.render_ios_springboard_frame(360, 800, &mut fb, &state);
        }
    }
}
