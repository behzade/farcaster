use x11rb::{
    connection::Connection as _,
    protocol::{
        shape::{ConnectionExt as _, SK, SO},
        xproto::{ClipOrdering, ConnectionExt as _, InputFocus},
    },
    rust_connection::RustConnection,
};

pub(super) fn configure_input(
    connection: &RustConnection,
    window: u32,
    modal: bool,
    tooltip: bool,
) -> Result<(), String> {
    if tooltip {
        connection
            .shape_rectangles(
                SO::SET,
                SK::INPUT,
                ClipOrdering::UNSORTED,
                window,
                0,
                0,
                &[],
            )
            .map_err(|error| error.to_string())?
            .check()
            .map_err(|error| error.to_string())?;
    } else {
        connection
            .shape_mask(SO::SET, SK::INPUT, window, 0, 0, x11rb::NONE)
            .map_err(|error| error.to_string())?
            .check()
            .map_err(|error| error.to_string())?;
    }
    if modal {
        // Override-redirect windows bypass the WM's normal focus policy.
        connection
            .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)
            .map_err(|error| error.to_string())?
            .check()
            .map_err(|error| error.to_string())?;
    }
    connection.flush().map_err(|error| error.to_string())
}
