pub trait Subtopic {
    fn subtopic<S: AsRef<str>>(&self, topic: S) -> String;
}

impl Subtopic for &str {
    fn subtopic<S: AsRef<str>>(&self, topic: S) -> String {
        format!("{}/{}", self, topic.as_ref())
    }
}
