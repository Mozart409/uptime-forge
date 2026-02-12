use std::collections::HashMap;

use maud::{DOCTYPE, Markup, html};

use crate::checker::CheckResult;
use crate::config::CheckType;
use crate::db::{BucketStatus, TimeRange};

/// Git hash at build time (set by build.rs)
pub const GIT_HASH: &str = env!("GIT_HASH");

/// Build timestamp (set by build.rs)
pub const BUILD_TIME: &str = env!("BUILD_TIME");

/// Base HTML layout that wraps page content
pub fn base(title: &str, content: &Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" class="h-full" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                link rel="icon" href="/favicon.svg";
                link rel="stylesheet" href="/css/output.css";
            }
            body class="bg-gray-100 min-h-full flex flex-col" {
                main class="flex-grow" {
                    (content)
                }
                (footer())
                script src="/js/htmx.min.js" defer {}
            }
        }
    }
}

/// Footer with build information
fn footer() -> Markup {
    html! {
        footer class="bg-gray-800 text-gray-400 py-4 mt-auto" {
            div class="container mx-auto px-4" {
                div class="flex flex-col sm:flex-row justify-between items-center gap-2 text-sm" {
                    div {
                        span class="font-semibold text-gray-300" { "Uptime Forge" }
                        span class="mx-2" { "|" }
                        span { "Built: " (BUILD_TIME) }
                    }
                    div class="flex items-center gap-2" {
                        span { "Commit: " }
                        a
                            href=(format!("https://github.com/Mozart409/uptime-forge/commit/{}", GIT_HASH))
                            target="_blank"
                            rel="noopener noreferrer"
                            class="font-mono text-blue-400 bg-gray-700 px-2 py-0.5 rounded hover:bg-gray-600 transition-colors"
                        {
                            (GIT_HASH)
                        }
                    }
                }
            }
        }
    }
}

/// Dashboard page showing endpoint status cards
pub fn dashboard(
    results: &[CheckResult],
    buckets: &HashMap<String, Vec<BucketStatus>>,
    time_range: TimeRange,
) -> Markup {
    let content = html! {
        div class="container mx-auto px-4 py-8" {
            header class="mb-8 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4" {
                div {
                    h1 class="text-3xl font-bold text-gray-800" { "Uptime Forge" }
                    p class="text-gray-600 mt-2" { "Endpoint Monitoring Dashboard" }
                }
                div class="flex items-center gap-4" {
                    // Time range dropdown
                    (time_range_dropdown(time_range))
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
            }

            main {
                // htmx polls /status every 10 seconds and swaps the content
                // hx-include references the dropdown so the current range is always sent
                div
                    id="status-grid"
                    hx-get="/status"
                    hx-trigger="every 10s"
                    hx-swap="innerHTML"
                    hx-include="#time-range-select"
                {
                    (status_grid_with_buckets(results, buckets, time_range))
                }
            }
        }
    };

    base("Uptime Forge - Dashboard", &content)
}

/// Time range dropdown selector
fn time_range_dropdown(current: TimeRange) -> Markup {
    html! {
        div class="relative" {
            select
                id="time-range-select"
                class="appearance-none bg-white border border-gray-300 rounded-lg px-4 py-2 pr-8 text-gray-700 cursor-pointer hover:border-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                hx-get="/status"
                hx-trigger="change"
                hx-target="#status-grid"
                hx-swap="innerHTML"
                name="range"
                hx-include="this"
            {
                @for range in TimeRange::all() {
                    option
                        value=(range.as_str())
                        selected[*range == current]
                    {
                        (range.label())
                    }
                }
            }
            // Dropdown arrow icon
            div class="pointer-events-none absolute inset-y-0 right-0 flex items-center px-2 text-gray-500" {
                svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" {
                    path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" {}
                }
            }
        }
    }
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

/// Grid of status cards with bucket data (partial for htmx updates)
pub fn status_grid_with_buckets(
    results: &[CheckResult],
    buckets: &HashMap<String, Vec<BucketStatus>>,
    time_range: TimeRange,
) -> Markup {
    html! {
        div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6" {
            @for result in results {
                @let endpoint_buckets = buckets.get(&result.name);
                (status_card_with_buckets(result, endpoint_buckets, time_range))
            }
        }

        @if results.is_empty() {
            p class="text-gray-500 text-center py-8" {
                "No endpoints configured. Add endpoints to forge.toml to start monitoring."
            }
        }
    }
}

/// Individual status card for an endpoint with status pills
fn status_card_with_buckets(
    result: &CheckResult,
    buckets: Option<&Vec<BucketStatus>>,
    time_range: TimeRange,
) -> Markup {
    let display_name = result.description.as_deref().unwrap_or(&result.name);

    let check_type_label = match result.check_type {
        CheckType::Http => "HTTP",
        CheckType::Tcp => "TCP",
        CheckType::Dns => "DNS",
    };

    html! {
        div class="bg-white rounded-lg shadow-md p-6 hover:shadow-lg transition-shadow" {
            div class="flex items-center justify-between mb-4" {
                div class="flex-1 min-w-0" {
                    h2 class="text-lg font-semibold text-gray-800 truncate" title=(display_name) {
                        (display_name)
                    }
                    // Show group if present
                    @if let Some(ref group) = result.group {
                        span class="text-xs text-gray-500" { (group) }
                    }
                }
                div class="flex items-center gap-2" {
                    // Check type badge
                    span class="px-2 py-0.5 text-xs font-medium bg-gray-100 text-gray-600 rounded" {
                        (check_type_label)
                    }
                    (status_indicator(result.is_up))
                }
            }

            // Tags
            @if !result.tags.is_empty() {
                div class="flex flex-wrap gap-1 mb-3" {
                    @for tag in &result.tags {
                        span class="px-2 py-0.5 text-xs bg-blue-100 text-blue-700 rounded" {
                            (tag)
                        }
                    }
                }
            }

            div class="space-y-2 text-sm" {
                div class="flex justify-between" {
                    span class="text-gray-500" { "Address" }
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
                        // Show error type badge if available
                        @if let Some(ref error_type) = result.error_type {
                            span class="inline-block px-1.5 py-0.5 bg-red-200 text-red-700 rounded text-xs font-medium mr-2" {
                                (error_type.as_str())
                            }
                        }
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

            // Status pills at the bottom
            (status_pills(buckets, time_range))
        }
    }
}

/// Status pills showing uptime history
fn status_pills(buckets: Option<&Vec<BucketStatus>>, time_range: TimeRange) -> Markup {
    html! {
        div class="mt-4 pt-4 border-t border-gray-100" {
            div class="flex items-center justify-between mb-2" {
                span class="text-xs text-gray-500" { "Uptime history" }
                span class="text-xs text-gray-400" { (time_range.label()) }
            }
            div class="flex gap-0.5" title="Status history (oldest to newest)" {
                @if let Some(bucket_list) = buckets {
                    @for bucket in bucket_list {
                        span class={"w-full h-2 rounded-sm " (bucket.css_class())} {}
                    }
                } @else {
                    // No data - show all gray pills
                    @for _ in 0..30 {
                        span class="w-full h-2 rounded-sm bg-gray-300" {}
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

/// Partial error message for htmx responses (e.g., database errors during polling)
pub fn db_error_partial(message: &str) -> Markup {
    html! {
        div class="bg-red-50 border border-red-200 rounded-lg p-6 text-center" {
            div class="flex items-center justify-center gap-3 text-red-600" {
                svg class="w-6 h-6" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" {
                    path stroke-linecap="round" stroke-linejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" {}
                }
                span class="font-medium" { (message) }
            }
        }
    }
}

/// Generic error page with customizable status code and message
pub fn error_page(status_code: u16, title: &str, message: &str) -> Markup {
    let (icon_color, bg_color) = match status_code {
        400..=499 => ("text-yellow-500", "bg-yellow-100"),
        500..=599 => ("text-red-500", "bg-red-100"),
        _ => ("text-gray-500", "bg-gray-100"),
    };

    let content = html! {
        div class="container mx-auto px-4 py-16" {
            div class="max-w-md mx-auto text-center" {
                // Error icon
                div class={"mx-auto w-24 h-24 rounded-full flex items-center justify-center mb-8 " (bg_color)} {
                    svg class={"w-12 h-12 " (icon_color)} xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" {
                        @if (400..500).contains(&status_code) {
                            // Question mark icon for 4xx
                            path stroke-linecap="round" stroke-linejoin="round" d="M9.879 7.519c1.171-1.025 3.071-1.025 4.242 0 1.172 1.025 1.172 2.687 0 3.712-.203.179-.43.326-.67.442-.745.361-1.45.999-1.45 1.827v.75M21 12a9 9 0 11-18 0 9 9 0 0118 0zm-9 5.25h.008v.008H12v-.008z" {}
                        } @else {
                            // Exclamation icon for 5xx and others
                            path stroke-linecap="round" stroke-linejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" {}
                        }
                    }
                }

                // Status code
                h1 class="text-6xl font-bold text-gray-800 mb-4" { (status_code) }

                // Title
                h2 class="text-2xl font-semibold text-gray-700 mb-4" { (title) }

                // Message
                p class="text-gray-600 mb-8" { (message) }

                // Back to home button
                a
                    href="/"
                    class="inline-flex items-center gap-2 px-6 py-3 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
                {
                    svg class="w-5 h-5" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" {
                        path stroke-linecap="round" stroke-linejoin="round" d="M2.25 12l8.954-8.955c.44-.439 1.152-.439 1.591 0L21.75 12M4.5 9.75v10.125c0 .621.504 1.125 1.125 1.125H9.75v-4.875c0-.621.504-1.125 1.125-1.125h2.25c.621 0 1.125.504 1.125 1.125V21h4.125c.621 0 1.125-.504 1.125-1.125V9.75M8.25 21h8.25" {}
                    }
                    "Back to Dashboard"
                }
            }
        }
    };

    base(&format!("{status_code} - {title}"), &content)
}
