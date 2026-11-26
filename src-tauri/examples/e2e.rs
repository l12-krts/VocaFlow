fn main() {
    let a: Vec<String> = std::env::args().collect();
    let format = a.get(4).map(|s| s == "format").unwrap_or(false);
    let ctx = [
        ("Mail", "Re: Q3 launch plan", "Hi Maria,\n\n"),
        ("Terminal", "zsh", "git commit -m \""),
        ("Google Chrome", "Home / X", ""),
        ("Slack", "#design - Slack", ""),
    ];
    vocaflow_lib::e2e::run(&a[1], &a[2], &a[3], &ctx, format).unwrap();
}
