//! Share a note through the OS share sheet (#105).
//!
//! The existing Markdown export (#79) writes a `.md` through a save dialog,
//! which is the right shape for "put this file somewhere" but the wrong one for
//! "send this to someone". This hands the note to the platform's own sharing UI
//! instead, so email, Messages/SMS, AirDrop and every other installed target
//! come for free rather than being enumerated by us.
//!
//! The note is shared as **plain text**, not as a file attachment. Mail drops it
//! into the message body and Messages sends it inline, which is what "share a
//! note via email or text" means in practice; a `.md` attachment would arrive as
//! an opaque blob in both. Callers wanting a file still have Export as Markdown.
//!
//! Both backends are best-effort and report a message rather than panicking —
//! there is no useful recovery from "the OS refused to show a share sheet", and
//! the frontend surfaces it like any other transfer error.

/// Title line prepended to the shared text, derived from the note path.
///
/// A bare Markdown body often starts mid-thought, and the receiving app shows no
/// filename, so the recipient gets no idea what they were sent. The note's own
/// `# Heading` is deliberately not reused: it may be absent, and duplicating it
/// when present reads worse than the filename.
fn subject_from_path(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .to_string()
}

/// Share `text` (the note's Markdown) with `subject` as its title.
///
/// `window` is only used on macOS, where a share sheet is a popover that must be
/// anchored to a view; Android's chooser is a full-screen activity.
pub fn share_note_text(
    #[allow(unused_variables)] window: &tauri::WebviewWindow,
    path: &str,
    text: &str,
) -> Result<(), String> {
    let subject = subject_from_path(path);

    #[cfg(target_os = "macos")]
    return macos::share(window, &subject, text);

    #[cfg(target_os = "android")]
    return android::share(&subject, text);

    #[cfg(not(any(target_os = "macos", target_os = "android")))]
    {
        let _ = (subject, text);
        Err("Sharing isn't supported on this platform yet".into())
    }
}

// macOS: NSSharingServicePicker, the same sheet Finder and Safari use.
#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_app_kit::{NSSharingServicePicker, NSWindow};
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString};

    pub fn share(window: &tauri::WebviewWindow, subject: &str, text: &str) -> Result<(), String> {
        // AppKit is main-thread-only, and Tauri commands run on a worker.
        let (subject, text) = (subject.to_string(), text.to_string());
        let w = window.clone();
        window
            .run_on_main_thread(move || show(&w, &subject, &text))
            .map_err(|e| format!("Couldn't reach the main thread: {e}"))
    }

    fn show(window: &tauri::WebviewWindow, subject: &str, text: &str) {
        let Ok(ptr) = window.ns_window() else { return };
        if ptr.is_null() {
            return;
        }
        // SAFETY: Tauri hands us a live NSWindow* for this window, and we are on
        // the main thread (run_on_main_thread above).
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        let Some(view) = ns.contentView() else { return };

        // Exactly ONE item. Every element of this array is treated as separate
        // *content*, not metadata — passing the subject as a second NSString made
        // the sheet announce "2 Images" and dropped Mail from the service list
        // entirely. A lone NSString is what makes Mail and Messages treat this as
        // a message body. macOS derives the mail subject from the text's first
        // line, which for a note is its `# Heading`; there is no way to set it
        // explicitly without implementing an NSSharingServiceDelegate.
        let _ = subject;
        let items: Retained<NSArray<AnyObject>> = NSArray::from_retained_slice(&[unsafe {
            Retained::cast_unchecked::<AnyObject>(NSString::from_str(text))
        }]);
        let picker =
            unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items) };

        // Anchor to the top-trailing corner of the window, under the header where
        // the settings button that opened this lives, so the popover points at
        // roughly where the click happened.
        let bounds = view.frame();
        const ANCHOR: f64 = 1.0;
        let rect = NSRect::new(
            NSPoint::new(bounds.size.width - 40.0, bounds.size.height - 40.0),
            NSSize::new(ANCHOR, ANCHOR),
        );
        picker.showRelativeToRect_ofView_preferredEdge(rect, &view, NSRectEdge::MinY);
    }
}

// Android: ACTION_SEND wrapped in a chooser, built through JNI.
//
// There is no Tauri plugin for this, and the Kotlin side has no hook we can call
// into, so the intent is assembled by hand. `ndk_context` holds the Application
// context (populated in lib.rs for iroh's network monitor) rather than the
// Activity, which is why the chooser needs FLAG_ACTIVITY_NEW_TASK.
#[cfg(target_os = "android")]
mod android {
    use jni::objects::{JObject, JValue};

    const ACTION_SEND: &str = "android.intent.action.SEND";
    const EXTRA_TEXT: &str = "android.intent.extra.TEXT";
    const EXTRA_SUBJECT: &str = "android.intent.extra.SUBJECT";
    const FLAG_ACTIVITY_NEW_TASK: i32 = 0x1000_0000;

    pub fn share(subject: &str, text: &str) -> Result<(), String> {
        run(subject, text).map_err(|e| format!("Couldn't open the share sheet: {e}"))
    }

    fn run(subject: &str, text: &str) -> Result<(), jni::errors::Error> {
        let ctx = ndk_context::android_context();
        // SAFETY: lib.rs initialises these from MainActivity.onCreate and leaks
        // the context global ref, so both stay valid for the process lifetime.
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }?;
        let mut env = vm.attach_current_thread()?;
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };

        // new Intent(ACTION_SEND).setType("text/plain")
        let action = env.new_string(ACTION_SEND)?;
        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&action)],
        )?;
        let mime = env.new_string("text/plain")?;
        env.call_method(
            &intent,
            "setType",
            "(Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&mime)],
        )?;

        // putExtra(EXTRA_TEXT, body) and putExtra(EXTRA_SUBJECT, title). Mail
        // apps read the subject; SMS apps ignore it and send the text alone.
        for (key, value) in [(EXTRA_SUBJECT, subject), (EXTRA_TEXT, text)] {
            let k = env.new_string(key)?;
            let v = env.new_string(value)?;
            env.call_method(
                &intent,
                "putExtra",
                "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&k), JValue::Object(&v)],
            )?;
        }

        // Intent.createChooser(intent, title) — always show the picker rather
        // than silently reusing a previously chosen default.
        let title = env.new_string(subject)?;
        let chooser = env
            .call_static_method(
                "android/content/Intent",
                "createChooser",
                "(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;",
                &[JValue::Object(&intent), JValue::Object(&title)],
            )?
            .l()?;
        // Required: an Application context has no task to place the activity in.
        env.call_method(
            &chooser,
            "addFlags",
            "(I)Landroid/content/Intent;",
            &[JValue::Int(FLAG_ACTIVITY_NEW_TASK)],
        )?;
        env.call_method(
            &context,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&chooser)],
        )?;
        Ok(())
    }
}

/// Read a note and hand it to the OS share sheet (#105).
///
/// Reads through the vault rather than taking the editor's buffer so what gets
/// shared is what is actually saved, and so the command works for any note path
/// rather than only the open one.
#[tauri::command]
pub async fn share_note(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::vault::VaultManager>,
    vault: String,
    path: String,
) -> Result<(), String> {
    let text = crate::vault::read_note(state, vault, path.clone()).await?;
    if text.trim().is_empty() {
        return Err("This note is empty — nothing to share".into());
    }
    share_note_text(&window, &path, &text)
}

#[cfg(test)]
mod tests {
    use super::subject_from_path;

    /// The title is what the recipient sees in a mail subject or chooser header,
    /// so it must survive folders, missing extensions and odd names.
    #[test]
    fn subject_is_the_note_name_without_extension() {
        assert_eq!(subject_from_path("ideas.md"), "ideas");
        assert_eq!(subject_from_path("journal/2026-08-09.md"), "2026-08-09");
        assert_eq!(subject_from_path("a/b/c/Deep Note.md"), "Deep Note");
        // No extension, and a name that merely contains ".md".
        assert_eq!(subject_from_path("notes/readme"), "readme");
        assert_eq!(subject_from_path("notes/md.md"), "md");
        // Degenerate inputs must not panic or produce an empty title silently.
        assert_eq!(subject_from_path(""), "");
    }
}
