//! Printing.
//!
//! Chromium's print preview and the system print dialog need a native
//! window, which offscreen browsers do not have, so Mori prints to PDF and
//! hands the file to the system viewer (which can print it). `window.print()`
//! takes the same path: [`PrintHandlerImpl`] reports the job as
//! [`ViewEvent::PrintRequested`] and gives Chromium no printer, so the
//! native job ends right there.

use std::{
    cell::RefCell,
    path::Path,
    rc::{Rc, Weak},
};

use cef::*;

use crate::view::{ViewEvent, ViewInner};

pub(crate) type PdfCallback = Box<dyn FnOnce(bool)>;

pub(crate) fn print_to_pdf(view: &Rc<ViewInner>, path: &Path, done: PdfCallback) {
    let Some(host) = view.host() else {
        done(false);
        return;
    };
    let settings = PdfPrintSettings {
        size: std::mem::size_of::<cef::sys::cef_pdf_print_settings_t>(),
        print_background: true as _,
        prefer_css_page_size: true as _,
        ..Default::default()
    };
    view.printing_pdf.set(true);
    let weak = Rc::downgrade(view);
    let done: PdfCallback = Box::new(move |ok| {
        if let Some(view) = weak.upgrade() {
            view.printing_pdf.set(false);
        }
        done(ok);
    });
    let mut callback = PdfDone::new(Rc::new(RefCell::new(Some(done))));
    host.print_to_pdf(
        Some(&CefString::from(path.to_string_lossy().as_ref())),
        Some(&settings),
        Some(&mut callback),
    );
}

wrap_pdf_print_callback! {
    pub(crate) struct PdfDone {
        done: Rc<RefCell<Option<PdfCallback>>>,
    }

    impl PdfPrintCallback {
        fn on_pdf_print_finished(&self, _path: Option<&CefString>, ok: ::std::os::raw::c_int) {
            let done = self.done.borrow_mut().take();
            if let Some(done) = done {
                done(ok == 1);
            }
        }
    }
}

pub(crate) fn build_handler(view: Weak<ViewInner>) -> PrintHandler {
    PrintHandlerImpl::new(view)
}

wrap_print_handler! {
    pub(crate) struct PrintHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl PrintHandler {
        fn on_print_start(&self, _browser: Option<&mut Browser>) {
            // The page asked to print (Mori's own PDF export prints too).
            if let Some(view) = self.view.upgrade()
                && !view.printing_pdf.get()
            {
                view.push(ViewEvent::PrintRequested);
            }
        }

        fn on_print_settings(
            &self,
            _browser: Option<&mut Browser>,
            settings: Option<&mut PrintSettings>,
            get_defaults: ::std::os::raw::c_int,
        ) {
            // A plausible default page lets Chromium reach the dialog step,
            // where Mori cancels: the job then ends like a user cancel
            // instead of failing half set up.
            let Some(settings) = settings.filter(|_| get_defaults == 1) else { return };
            if settings.is_read_only() == 1 {
                return;
            }
            const DPI: i32 = 72;
            let (w, h) = (DPI * 17 / 2, DPI * 11); // US Letter
            settings.set_device_name(Some(&CefString::from("Mori")));
            settings.set_dpi(DPI);
            settings.set_printer_printable_area(
                Some(&Size { width: w, height: h }),
                Some(&Rect { x: 0, y: 0, width: w, height: h }),
                0,
            );
        }

        fn on_print_dialog(
            &self,
            _browser: Option<&mut Browser>,
            _has_selection: ::std::os::raw::c_int,
            callback: Option<&mut PrintDialogCallback>,
        ) -> ::std::os::raw::c_int {
            if let Some(callback) = callback {
                callback.cancel();
            }
            1
        }

        fn on_print_job(
            &self,
            _browser: Option<&mut Browser>,
            _document_name: Option<&CefString>,
            _pdf_file_path: Option<&CefString>,
            callback: Option<&mut PrintJobCallback>,
        ) -> ::std::os::raw::c_int {
            // Only reached after a confirmed dialog, which Mori never shows.
            if let Some(callback) = callback {
                callback.cont();
            }
            1
        }
    }
}
