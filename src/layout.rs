use maud::{DOCTYPE, Markup, html};

use crate::checker::CheckResult;

/// Base HTML layout that wraps page content
pub fn base(title: &str, content: &Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                link rel="icon" href="/favicon.svg";
                link rel="stylesheet" href="/css/output.css";
            }
            body class="bg-gray-100 min-h-screen" {
                (content)
                script src="/js/htmx.min.js" defer {}
            }
        }
    }
}

/// Dashboard page showing endpoint status cards
pub fn dashboard(results: &[CheckResult]) -> Markup {
    let content = html! {
        div class="container mx-auto px-4 py-8" {
            header class="mb-8 flex items-center justify-between" {
                div {
                    h1 class="text-3xl font-bold text-gray-800" { "Uptime Forge" }
                    p class="text-gray-600 mt-2" { "Endpoint Monitoring Dashboard" }
                }
                button
                    class="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors flex items-center gap-2"
                    hx-get="/reload"
                    hx-swap="none"
                    hx-indicator="#reload-spinner"
                {
                    span id="reload-spinner" class="htmx-indicator" {
                        (spinner())
                    }
                    "Reload Config"
                }
            }

            main {
                // htmx polls /status every 10 seconds and swaps the content
                div
                    id="status-grid"
                    hx-get="/status"
                    hx-trigger="every 10s"
                    hx-swap="innerHTML"
                {
                    (status_grid(results))
                }
            }
        }
    };

    base("Uptime Forge - Dashboard", &content)
}

/// Loading spinner for htmx requests
fn spinner() -> Markup {
    html! {
        svg class="animate-spin h-4 w-4" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" {
            circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" {}
            path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" {}
        }
    }
}

/// Grid of status cards (partial for htmx updates)
pub fn status_grid(results: &[CheckResult]) -> Markup {
    html! {
        div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6" {
            @for result in results {
                (status_card(result))
            }
        }

        @if results.is_empty() {
            p class="text-gray-500 text-center py-8" {
                "No endpoints configured. Add endpoints to forge.toml to start monitoring."
            }
        }
    }
}

/// Individual status card for an endpoint
fn status_card(result: &CheckResult) -> Markup {
    let display_name = result.description.as_deref().unwrap_or(&result.name);

    html! {
        div class="bg-white rounded-lg shadow-md p-6 hover:shadow-lg transition-shadow" {
            div class="flex items-center justify-between mb-4" {
                h2 class="text-lg font-semibold text-gray-800 truncate" title=(display_name) {
                    (display_name)
                }
                (status_indicator(result.is_up))
            }

            div class="space-y-2 text-sm" {
                div class="flex justify-between" {
                    span class="text-gray-500" { "URL" }
                    span class="text-gray-700 truncate ml-2 max-w-[200px]" title=(result.addr) {
                        (result.addr)
                    }
                }

                @if let Some(status) = result.status_code {
                    div class="flex justify-between" {
                        span class="text-gray-500" { "Status" }
                        span class="text-gray-700" { (status) }
                    }
                }

                @if let Some(ms) = result.response_time_ms {
                    div class="flex justify-between" {
                        span class="text-gray-500" { "Response" }
                        span class="text-gray-700" { (ms) "ms" }
                    }
                }

                @if let Some(ref error) = result.error {
                    div class="mt-3 p-2 bg-red-50 rounded text-red-600 text-xs" {
                        (error)
                        @if let Some(status) = result.status_code {
                            @if !result.is_up {
                                " "
                                a
                                    href=(format!("https://http.cat/{}", status))
                                    target="_blank"
                                    rel="noopener noreferrer"
                                    class="inline-flex items-center gap-1 ml-1 px-2 py-0.5 bg-red-200 text-red-700 rounded font-medium hover:bg-red-300 transition-colors"
                                {
                                    "http.cat"
                                    // External link icon
                                    svg class="w-3 h-3" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor" {
                                        path stroke-linecap="round" stroke-linejoin="round" d="M13.5 6H5.25A2.25 2.25 0 003 8.25v10.5A2.25 2.25 0 005.25 21h10.5A2.25 2.25 0 0018 18.75V10.5m-10.5 6L21 3m0 0h-5.25M21 3v5.25" {}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Pulsing status indicator dot
fn status_indicator(is_up: bool) -> Markup {
    let (bg_color, pulse_color) = if is_up {
        ("bg-green-500", "bg-green-400")
    } else {
        ("bg-red-500", "bg-red-400")
    };

    html! {
        span class="relative flex h-3 w-3" {
            span class={"animate-ping absolute inline-flex h-full w-full rounded-full opacity-75 " (pulse_color)} {}
            span class={"relative inline-flex rounded-full h-3 w-3 " (bg_color)} {}
        }
    }
}
