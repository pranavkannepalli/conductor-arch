use gtk::prelude::*;
use gtk::{Box as GBox, ListBox, ListBoxRow, Revealer, RevealerTransitionType, Widget};
use std::time::Duration;

pub(crate) const ENTER_EXIT_MS: u32 = 180;

pub(crate) fn revealer_for<W: IsA<Widget>>(child: &W) -> Revealer {
    let revealer = Revealer::new();
    revealer.set_transition_type(RevealerTransitionType::SlideDown);
    revealer.set_transition_duration(ENTER_EXIT_MS);
    revealer.set_reveal_child(false);
    revealer.set_child(Some(child));
    revealer
}

pub(crate) fn append_revealed<W: IsA<Widget>>(container: &GBox, child: &W) -> Revealer {
    let revealer = revealer_for(child);
    container.append(&revealer);
    reveal_on_next_tick(&revealer);
    revealer
}

pub(crate) fn append_revealed_to_list<W: IsA<Widget>>(container: &ListBox, child: &W) -> Revealer {
    let revealer = revealer_for(child);
    container.append(&revealer);
    reveal_on_next_tick(&revealer);
    revealer
}

pub(crate) fn append_revealed_row(container: &ListBox, row: &ListBoxRow) -> Option<Revealer> {
    let child = row.child()?;
    row.set_child(None::<&Widget>);
    let revealer = revealer_for(&child);
    row.set_child(Some(&revealer));
    container.append(row);
    reveal_on_next_tick(&revealer);
    Some(revealer)
}

pub(crate) fn clear_box(container: &GBox) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

pub(crate) fn clear_list(container: &ListBox) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

pub(crate) fn remove_revealed<W: IsA<Widget>>(container: &GBox, child: &W) {
    let widget = child.as_ref().clone();
    if let Ok(revealer) = widget.clone().downcast::<Revealer>() {
        revealer.set_reveal_child(false);
        let container = container.clone();
        glib::timeout_add_local_once(Duration::from_millis(u64::from(ENTER_EXIT_MS)), move || {
            container.remove(&revealer);
        });
    } else {
        container.remove(&widget);
    }
}

fn reveal_on_next_tick(revealer: &Revealer) {
    let revealer = revealer.clone();
    glib::idle_add_local_once(move || {
        revealer.set_reveal_child(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_motion_duration_stays_short_enough_for_row_refreshes() {
        assert!((120..=240).contains(&ENTER_EXIT_MS));
    }
}
