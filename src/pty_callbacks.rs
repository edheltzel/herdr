//! Terminal query responses.
//!
//! Programs like fzf, htop, and vim send escape sequences asking "what terminal am I?"
//! and "where's the cursor?". A real terminal responds immediately. We need to do the same.
//!
//! There are ~15 query types that real programs use. We handle the common ones.
//! The vt100 parser handles all OUTPUT sequences (formatting, cursor, etc.) — those
//! don't need responses. Only QUERY sequences need us to reply.

use std::sync::{
    atomic::{AtomicBool, AtomicU16, Ordering},
    Arc, Mutex,
};

/// Collects response bytes that need to be written back to the PTY.
/// Also tracks whether the child has requested the Kitty keyboard protocol.
#[derive(Clone, Default)]
pub struct PtyResponses {
    pending: Arc<Mutex<Vec<u8>>>,
    /// Stack of kitty keyboard enhancement flags pushed by child programs.
    /// Each push adds a flags value; pop removes the top. This correctly handles
    /// nested programs (e.g. fish pushes mode 1, neovim pushes mode 3, neovim
    /// pops → fish's mode 1 is restored).
    kitty_stack: Arc<Mutex<Vec<u16>>>,
    /// Derived from kitty_stack: true when stack is non-empty.
    /// Kept for quick feature checks.
    pub kitty_keyboard: Arc<AtomicBool>,
    /// Exact active kitty keyboard flags from the top of the stack.
    pub kitty_keyboard_flags: Arc<AtomicU16>,
}

impl PtyResponses {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take any pending response bytes (clears the buffer).
    pub fn take(&self) -> Vec<u8> {
        let mut pending = self.pending.lock().unwrap();
        std::mem::take(&mut *pending)
    }

    fn push(&self, bytes: &[u8]) {
        self.pending.lock().unwrap().extend_from_slice(bytes);
    }
}

impl vt100::Callbacks for PtyResponses {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let param0 = params.first().and_then(|p| p.first()).copied().unwrap_or(0);

        match (i1, c) {
            // === Device Attributes ===

            // DA1: \e[c or \e[0c → "what terminal are you?"
            // Respond as VT220 with ANSI color
            (None, 'c') if param0 == 0 => {
                self.push(b"\x1b[?62;22c");
            }

            // DA2: \e[>c or \e[>0c → "secondary device attributes"
            (Some(b'>'), 'c') => {
                // Type 0 (VT100), firmware version 0, ROM version 0
                self.push(b"\x1b[>0;0;0c");
            }

            // === Cursor / Status Reports ===

            // DSR: \e[Nn where N selects the report type
            (None, 'n') => match param0 {
                // CPR: \e[6n → cursor position report
                5 => {
                    // Device status: "OK"
                    self.push(b"\x1b[0n");
                }
                6 => {
                    let (row, col) = screen.cursor_position();
                    let response = format!("\x1b[{};{}R", row + 1, col + 1);
                    self.push(response.as_bytes());
                }
                _ => {}
            },

            // DECXCPR: \e[?6n → extended cursor position report (with page)
            (Some(b'?'), 'n') if param0 == 6 => {
                let (row, col) = screen.cursor_position();
                let response = format!("\x1b[?{};{}R", row + 1, col + 1);
                self.push(response.as_bytes());
            }

            // === Mode Queries (DECRQM) ===

            // DECRQM: \e[?Np → "is DEC private mode N set?"
            // Response: \e[?N;Ps$y where Ps = 1 (set), 2 (reset), 0 (unknown)
            (Some(b'?'), 'p') => {
                // Report all queried modes as "reset" (2) — safe default
                let response = format!("\x1b[?{param0};2$y");
                self.push(response.as_bytes());
            }

            // ANSI DECRQM: \e[Np → "is ANSI mode N set?"
            (None, 'p') => {
                let response = format!("\x1b[{param0};2$y");
                self.push(response.as_bytes());
            }

            // === Keyboard Protocol ===

            // Kitty keyboard query: \e[?u → "what keyboard flags are active?"
            (Some(b'?'), 'u') => {
                let stack = self.kitty_stack.lock().unwrap();
                let flags = stack.last().copied().unwrap_or(0);
                self.push(format!("\x1b[?{flags}u").as_bytes());
            }

            // Kitty keyboard push: \e[>Nu → child wants Kitty key encoding
            (Some(b'>'), 'u') => {
                let mut stack = self.kitty_stack.lock().unwrap();
                stack.push(param0);
                self.kitty_keyboard.store(true, Ordering::Relaxed);
                self.kitty_keyboard_flags.store(param0, Ordering::Relaxed);
            }

            // Kitty keyboard pop: \e[<Nu → child reverts to previous mode
            // N = number of entries to pop (default 1)
            (Some(b'<'), 'u') => {
                let mut stack = self.kitty_stack.lock().unwrap();
                let count = (param0 as usize).max(1);
                for _ in 0..count {
                    if stack.pop().is_none() {
                        break;
                    }
                }
                let flags = stack.last().copied().unwrap_or(0);
                self.kitty_keyboard
                    .store(!stack.is_empty(), Ordering::Relaxed);
                self.kitty_keyboard_flags.store(flags, Ordering::Relaxed);
            }

            // === Terminal Identification ===

            // XTVERSION: \e[>q → "what terminal version?"
            (Some(b'>'), 'q') => {
                // Respond in DCS format: \eP>|herdr 0.1\e\\
                self.push(b"\x1bP>|herdr 0.1\x1b\\");
            }

            _ => {}
        }
    }

    fn unhandled_osc(&mut self, _screen: &mut vt100::Screen, params: &[&[u8]]) {
        let Some(cmd) = params.first() else { return };

        match *cmd {
            // OSC 10 ; ? ST → query foreground color
            b"10" => {
                if params.get(1) == Some(&&b"?"[..]) {
                    // Respond with a default light foreground
                    self.push(b"\x1b]10;rgb:cccc/cccc/cccc\x1b\\");
                }
            }
            // OSC 11 ; ? ST → query background color
            b"11" => {
                if params.get(1) == Some(&&b"?"[..]) {
                    // Respond with a default dark background
                    self.push(b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\");
                }
            }
            // OSC 12 ; ? ST → query cursor color
            b"12" => {
                if params.get(1) == Some(&&b"?"[..]) {
                    self.push(b"\x1b]12;rgb:cccc/cccc/cccc\x1b\\");
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_parser(responses: PtyResponses) -> vt100::Parser<PtyResponses> {
        vt100::Parser::new_with_callbacks(24, 80, 0, responses)
    }

    #[test]
    fn responds_to_da1() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b[c");
        assert_eq!(r.take(), b"\x1b[?62;22c");
    }

    #[test]
    fn responds_to_da1_explicit_zero() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b[0c");
        assert_eq!(r.take(), b"\x1b[?62;22c");
    }

    #[test]
    fn responds_to_da2() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b[>c");
        assert_eq!(r.take(), b"\x1b[>0;0;0c");
    }

    #[test]
    fn responds_to_cpr() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b[5;10H"); // move cursor to row 5, col 10
        p.process(b"\x1b[6n");
        assert_eq!(r.take(), b"\x1b[5;10R");
    }

    #[test]
    fn responds_to_dsr_status() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b[5n"); // device status report
        assert_eq!(r.take(), b"\x1b[0n"); // "OK"
    }

    #[test]
    fn responds_to_extended_cpr() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b[3;7H"); // move cursor
        p.process(b"\x1b[?6n"); // extended CPR
        assert_eq!(r.take(), b"\x1b[?3;7R");
    }

    #[test]
    fn responds_to_decrqm_private() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b[?25p"); // query: is cursor visible (mode 25)?
        assert_eq!(r.take(), b"\x1b[?25;2$y"); // "reset" (2)
    }

    #[test]
    fn responds_to_kitty_keyboard_query() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b[?u");
        assert_eq!(r.take(), b"\x1b[?0u");
    }

    #[test]
    fn kitty_push_pop_stack() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());

        // Initially off
        assert!(!r.kitty_keyboard.load(Ordering::Relaxed));
        assert_eq!(r.kitty_keyboard_flags.load(Ordering::Relaxed), 0);

        // Fish pushes mode 1
        p.process(b"\x1b[>1u");
        assert!(r.kitty_keyboard.load(Ordering::Relaxed));
        assert_eq!(r.kitty_keyboard_flags.load(Ordering::Relaxed), 1);
        p.process(b"\x1b[?u");
        assert_eq!(r.take(), b"\x1b[?1u");

        // Neovim pushes mode 3
        p.process(b"\x1b[>3u");
        assert!(r.kitty_keyboard.load(Ordering::Relaxed));
        assert_eq!(r.kitty_keyboard_flags.load(Ordering::Relaxed), 3);
        p.process(b"\x1b[?u");
        assert_eq!(r.take(), b"\x1b[?3u");

        // Neovim pops → fish's mode 1 is restored
        p.process(b"\x1b[<u");
        assert!(r.kitty_keyboard.load(Ordering::Relaxed));
        assert_eq!(r.kitty_keyboard_flags.load(Ordering::Relaxed), 1);
        p.process(b"\x1b[?u");
        assert_eq!(r.take(), b"\x1b[?1u");

        // Fish pops → back to legacy
        p.process(b"\x1b[<u");
        assert!(!r.kitty_keyboard.load(Ordering::Relaxed));
        assert_eq!(r.kitty_keyboard_flags.load(Ordering::Relaxed), 0);
        p.process(b"\x1b[?u");
        assert_eq!(r.take(), b"\x1b[?0u");
    }

    #[test]
    fn kitty_pop_on_empty_is_harmless() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());

        // Pop with nothing on stack — should not crash
        p.process(b"\x1b[<u");
        assert!(!r.kitty_keyboard.load(Ordering::Relaxed));
        assert_eq!(r.kitty_keyboard_flags.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn kitty_pop_count() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());

        // Push three times
        p.process(b"\x1b[>1u");
        p.process(b"\x1b[>3u");
        p.process(b"\x1b[>5u");
        assert!(r.kitty_keyboard.load(Ordering::Relaxed));
        assert_eq!(r.kitty_keyboard_flags.load(Ordering::Relaxed), 5);

        // Pop 2 at once
        p.process(b"\x1b[<2u");
        assert!(r.kitty_keyboard.load(Ordering::Relaxed));
        assert_eq!(r.kitty_keyboard_flags.load(Ordering::Relaxed), 1);
        p.process(b"\x1b[?u");
        assert_eq!(r.take(), b"\x1b[?1u"); // only first push remains

        // Pop last one
        p.process(b"\x1b[<u");
        assert!(!r.kitty_keyboard.load(Ordering::Relaxed));
        assert_eq!(r.kitty_keyboard_flags.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn responds_to_xtversion() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b[>q");
        assert_eq!(r.take(), b"\x1bP>|herdr 0.1\x1b\\");
    }

    #[test]
    fn responds_to_osc_fg_color_query() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b]10;?\x1b\\");
        assert_eq!(r.take(), b"\x1b]10;rgb:cccc/cccc/cccc\x1b\\");
    }

    #[test]
    fn responds_to_osc_bg_color_query() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b]11;?\x1b\\");
        assert_eq!(r.take(), b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\");
    }

    #[test]
    fn responds_to_osc_cursor_color_query() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b]12;?\x1b\\");
        assert_eq!(r.take(), b"\x1b]12;rgb:cccc/cccc/cccc\x1b\\");
    }

    #[test]
    fn no_response_for_regular_output() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"hello world\x1b[31mred\x1b[0m");
        assert!(r.take().is_empty());
    }

    #[test]
    fn multiple_queries_accumulate() {
        let r = PtyResponses::new();
        let mut p = make_parser(r.clone());
        p.process(b"\x1b[c\x1b[6n");
        let bytes = r.take();
        assert!(bytes.starts_with(b"\x1b[?62;22c"));
        assert!(bytes.ends_with(b"\x1b[1;1R"));
    }
}
