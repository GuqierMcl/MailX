//! Local drafts: storage-independent types, validation and reply / forward
//! generation. Persistence lives in `storage::drafts`; the service methods are
//! on `AccountService` and `MailxCore`.
pub mod attachments;
pub(crate) mod body;
pub(crate) mod compose;
pub mod types;
pub(crate) mod validate;

#[cfg(test)]
mod tests;
