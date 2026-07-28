mod platform;

pub fn send_notification(title: &str, message: &str) {
    if platform::send_notification(title, message) {
        return;
    }

    println!("{title}: {message}");
}
