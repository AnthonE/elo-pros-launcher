//! A row's download meter is built down, and built where the words are.
//!
//! The meter stands in for a row's one status sentence while bytes move
//! (`wiring::drive_meter`), so two properties are load-bearing enough to pin
//! at construction:
//!
//! * **it is hidden until someone has bytes to report** — a meter at rest is
//!   a promise of motion the row cannot keep, and the sentence it covers
//!   (status line, price line) must be the thing on screen instead;
//! * **it stays out of the controls' column** — a bar drawn under a button is
//!   a bar the player reads as part of the button.
//!
//! ⚠ One widget-building test per file — FLTK pins itself to the first thread
//! that constructs a widget and the harness gives each `#[test]` its own, so
//! a second widget test in this binary dies on `is_ui_thread()`. Same shape as
//! `shelves.rs`, `credit.rs`, `shards.rs`. Split rather than add.
//!
//! FLTK needs no display to construct widgets, so all of this runs headless.

use fltk::prelude::*;
use scry_ui::windows::{self, Price, Row, Shelf};

#[test]
fn every_meter_is_built_down_beside_its_words_and_off_the_buttons() {
    let row = Row {
        slug: "gates".into(),
        build: "0.1.0".into(),
        bytes: 75_829_730,
        digest: "0xf4".into(),
        stale: Some(true),
        name: Some("Gates".into()),
        icon: None,
        why: None,
    };
    let games = windows::games(&[row]);
    let g = &games.rows[0];
    assert!(!g.progress.visible(), "a games meter must be down until bytes move");
    assert!(g.status.visible(), "the status line is what shows until then");
    assert_eq!(g.progress.maximum(), 100.0, "the fill is a percentage");
    assert_eq!((g.progress.x(), g.progress.y()), (g.status.x(), g.status.y()),
               "the meter stands exactly where the sentence it replaces stands");
    assert!(g.progress.x() + g.progress.w() < g.act.x(),
            "the meter must not run under the row's buttons");

    let shelf = Shelf {
        slug: "gates".into(),
        name: "Gates".into(),
        blurb: "survival".into(),
        state: "live".into(),
        installable: true,
        installed: false,
        price: Price::Free,
        icon: None,
    };
    let store = windows::store(&[shelf], true, "");
    let s = &store.rows[0];
    assert!(!s.progress.visible(), "a store meter must be down until bytes move");
    assert!(s.price.visible(), "the price line is what shows until then");
    assert_eq!(s.progress.maximum(), 100.0);
    assert_eq!((s.progress.x(), s.progress.y()), (s.price.x(), s.price.y()),
               "the meter stands exactly where the sentence it replaces stands");
    assert!(s.progress.x() + s.progress.w() < s.button.x(),
            "the meter must not run under the row's buttons");
}
