use phantom_core::{Result, implement, rand, warn};
use phantom_database::Txn;
use reqwest::Url;
use webpage::OpengraphObject;

use crate::media::{MXC_LENGTH, Service};

#[implement(Service)]
pub(in crate::media) async fn forget_lazy_media(&self, mxc: &str) -> Result<bool> {
    if self.db.search_lazy_media(mxc).await.is_err() {
        return Ok(false);
    }

    let mut txn = self.db.txn();

    self.db.remove_lazy_media(&mut txn, mxc);
    self.db.remove_lazy_content(&mut txn, mxc);

    txn.execute()?;

    Ok(true)
}

#[implement(Service)]
pub(super) fn register_lazy_media(&self, url: &str) -> Result<String> {
    let mxc = self.mint_lazy_media();

    self.db.insert_lazy_media(&mxc, url)?;

    Ok(mxc)
}

#[implement(Service)]
pub(super) fn queue_lazy_media(&self, txn: &mut Txn, url: &str) -> String {
    let mxc = self.mint_lazy_media();

    self.db.queue_lazy_media(txn, &mxc, url);

    mxc
}

#[implement(Service)]
fn mint_lazy_media(&self) -> String {
    let server_name = self.services.server_state.server_name();
    let media_id = rand::string(MXC_LENGTH);

    format!("mxc://{server_name}/{media_id}")
}

#[implement(Service)]
pub(super) fn lazy_media(&self, page: &Url, obj: &OpengraphObject, class: &str) -> Option<String> {
    declares_media_type(obj, class)
        .then(|| page.join(&obj.url).ok())
        .flatten()
        .filter(|url| ["http", "https"].contains(&url.scheme()))
        .filter(|url| self.check_url_host(url).is_ok())
        .and_then(|url| {
            self.register_lazy_media(url.as_str())
                .inspect_err(|e| warn!(%url, "Could not register preview media: {e}"))
                .ok()
        })
}

fn declares_media_type(obj: &OpengraphObject, class: &str) -> bool {
    obj.properties
        .get("type")
        .is_none_or(|kind| kind.starts_with(class))
}
