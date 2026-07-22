use gtk::prelude::*;

pub(crate) fn configure_column_header<W: IsA<gtk::Widget>>(header: &W) {
    header.set_height_request(crate::COLUMN_HEADER_HEIGHT);
    header.set_vexpand(false);
    header.set_valign(gtk::Align::Center);
    header.set_overflow(gtk::Overflow::Hidden);

    let gesture = gtk::GestureClick::new();
    gesture.set_button(1);
    gesture.connect_pressed(move |gesture, _presses, x, y| {
        let Some(event) = gesture.current_event() else {
            return;
        };
        let Some(device) = event.device() else {
            return;
        };
        let Some(widget) = gesture.widget() else {
            return;
        };
        let Some(root) = widget.root() else {
            return;
        };
        let Some(point) = widget.compute_point(
            root.upcast_ref::<gtk::Widget>(),
            &gtk::graphene::Point::new(x as f32, y as f32),
        ) else {
            return;
        };
        let Some(surface) = widget.native().and_then(|native| native.surface()) else {
            return;
        };
        let Ok(toplevel) = surface.downcast::<gtk::gdk::Toplevel>() else {
            return;
        };
        toplevel.begin_move(&device, 1, point.x() as f64, point.y() as f64, event.time());
    });
    header.add_controller(gesture);
}
