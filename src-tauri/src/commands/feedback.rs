use tauri::{
    webview::NewWindowResponse, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

const FEEDBACK_WINDOW: &str = "feedback";

fn is_https(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}

fn is_feedback_url(url: &Url) -> bool {
    is_https(url)
        || (cfg!(debug_assertions)
            && url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            && url.username().is_empty()
            && url.password().is_none())
}

fn allow_navigation(target: &Url, form: &Url) -> bool {
    target.origin() == form.origin()
        || matches!(target.as_str(), "about:blank" | "about:srcdoc")
        // WKWebView also applies the navigation policy to the Turnstile frame.
        || (is_https(target)
            && target.host_str() == Some("challenges.cloudflare.com")
            && target.port().is_none()
            && target.path().starts_with("/cdn-cgi/challenge-platform/"))
}

fn open_issue_link(target: &Url) {
    const ISSUES_PATH: &str = "/ZackaryShen/ColorYourModel/issues";
    if is_https(target)
        && target.host_str() == Some("github.com")
        && target.port().is_none()
        && (target.path() == ISSUES_PATH
            || target
                .path()
                .strip_prefix(ISSUES_PATH)
                .is_some_and(|suffix| {
                    suffix.strip_prefix('/').is_some_and(|number| {
                        !number.is_empty() && number.bytes().all(|digit| digit.is_ascii_digit())
                    })
                }))
    {
        if let Err(error) = tauri_plugin_opener::open_url(target.as_str(), None::<&str>) {
            log::warn!("Could not open feedback issue link: {error}");
        }
    }
}

#[tauri::command]
pub async fn open_feedback_window(
    window: WebviewWindow,
    url: String,
    title: String,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Feedback must be opened from the main window".into());
    }
    let app = window.app_handle();
    if let Some(feedback) = app.get_webview_window(FEEDBACK_WINDOW) {
        feedback.show().map_err(|error| error.to_string())?;
        return feedback.set_focus().map_err(|error| error.to_string());
    }

    let form = Url::parse(&url).map_err(|error| error.to_string())?;
    if !is_feedback_url(&form)
        || window
            .url()
            .is_ok_and(|main| main.origin() == form.origin())
    {
        return Err("Invalid feedback URL".into());
    }

    WebviewWindowBuilder::new(app, FEEDBACK_WINDOW, WebviewUrl::External(form.clone()))
        .parent(&window)
        .map_err(|error| error.to_string())?
        .title(title)
        .inner_size(600.0, 700.0)
        .min_inner_size(390.0, 430.0)
        .center()
        .minimizable(false)
        .skip_taskbar(true)
        .on_navigation(move |target| {
            if allow_navigation(target, &form) {
                return true;
            }
            open_issue_link(target);
            false
        })
        .on_new_window(|target, _| {
            open_issue_link(&target);
            NewWindowResponse::Deny
        })
        .build()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_webview_stays_on_its_form_and_turnstile() {
        let form = Url::parse("https://feedback.example.com/?lang=zh").unwrap();
        for allowed in [
            "https://feedback.example.com/submit",
            "about:blank",
            "about:srcdoc",
            "https://challenges.cloudflare.com/cdn-cgi/challenge-platform/h/b/turnstile/",
        ] {
            assert!(allow_navigation(&Url::parse(allowed).unwrap(), &form));
        }
        for blocked in [
            "http://feedback.example.com/submit",
            "https://feedback.example.com:444/submit",
            "https://feedback.example.com.evil.test/submit",
            "https://challenges.cloudflare.com/",
            "https://github.com/ZackaryShen/ColorYourModel/issues/40",
            "tauri://localhost/",
        ] {
            assert!(!allow_navigation(&Url::parse(blocked).unwrap(), &form));
        }
    }
}
