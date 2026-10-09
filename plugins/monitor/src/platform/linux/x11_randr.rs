use std::fmt;

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::randr;
use x11rb::rust_connection::RustConnection;

use crate::monitor::backends::x11_randr_gamma::{
    connector_output_suffix, GammaBus, GammaError, GammaTable, GammaTransport,
};

pub struct X11GammaTransport;

pub struct X11GammaBus {
    conn: RustConnection,
    screen: usize,
}

impl GammaTransport for X11GammaTransport {
    type Bus = X11GammaBus;

    fn open(&self) -> Result<Self::Bus, GammaError> {
        let (conn, screen) = x11rb::connect(None).map_err(|error| GammaError::Unsupported {
            detail: format!("cannot connect to the X11 server: {error}"),
        })?;
        Ok(X11GammaBus { conn, screen })
    }
}

impl GammaBus for X11GammaBus {
    fn crtc_for_connector(&mut self, connector: &str) -> Result<Option<u32>, GammaError> {
        self.conn
            .extension_information(randr::X11_EXTENSION_NAME)
            .map_err(x11_error)?
            .ok_or_else(|| GammaError::Unsupported {
                detail: "the RandR extension is not available on this X11 server".into(),
            })?;
        let Some(root) = self
            .conn
            .setup()
            .roots
            .get(self.screen)
            .map(|screen| screen.root)
        else {
            return Err(GammaError::Unsupported {
                detail: "the X11 setup carries no screen".into(),
            });
        };
        let resources = randr::get_screen_resources_current(&self.conn, root)
            .map_err(x11_error)?
            .reply()
            .map_err(x11_error)?;
        let Some(suffix) = connector_output_suffix(connector) else {
            return Ok(None);
        };
        for output in resources.outputs {
            let output_info =
                randr::get_output_info(&self.conn, output, resources.config_timestamp)
                    .map_err(x11_error)?
                    .reply()
                    .map_err(x11_error)?;
            if output_info.connection != randr::Connection::CONNECTED || output_info.crtc == 0 {
                continue;
            }
            if String::from_utf8_lossy(&output_info.name) == suffix {
                return Ok(Some(output_info.crtc));
            }
        }
        Ok(None)
    }

    fn read_gamma(&mut self, crtc: u32) -> Result<GammaTable, GammaError> {
        let reply = randr::get_crtc_gamma(&self.conn, crtc)
            .map_err(x11_error)?
            .reply()
            .map_err(x11_error)?;
        Ok(GammaTable {
            red: reply.red,
            green: reply.green,
            blue: reply.blue,
        })
    }

    fn write_gamma(&mut self, crtc: u32, table: &GammaTable) -> Result<(), GammaError> {
        randr::set_crtc_gamma(&self.conn, crtc, &table.red, &table.green, &table.blue)
            .map_err(x11_error)?
            .check()
            .map_err(x11_error)
    }

    fn hdr_active(&mut self, _crtc: u32) -> Result<bool, GammaError> {
        Err(GammaError::Unsupported {
            detail: "HDR state is not readable through RandR on X11".into(),
        })
    }
}

fn x11_error(error: impl fmt::Display) -> GammaError {
    GammaError::Unsupported {
        detail: format!("X11 RandR request failed: {error}"),
    }
}
