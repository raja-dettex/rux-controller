

pub trait Resource: Send + Sync + 'static {
    type Spec: Send + Sync + Clone;
    type Status: Send + Sync + Clone;

    fn key(&self) -> String;
}