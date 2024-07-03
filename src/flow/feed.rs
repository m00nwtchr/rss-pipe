use async_trait::async_trait;
use rss::Channel;
use serde::{Deserialize, Serialize};
use url::Url;

use super::node::NodeTrait;

#[derive(Serialize, Deserialize, Debug)]
pub struct Feed {
	url: Url,
}

impl Feed {
	pub fn new(url: Url) -> Self {
		Self { url }
	}
}

#[async_trait]
impl NodeTrait for Feed {
	type Item = Channel;

	#[tracing::instrument(name = "feed_node")]
	async fn run(&self) -> anyhow::Result<Channel> {
		let content = reqwest::get(self.url.clone()).await?.bytes().await?;
		let channel = Channel::read_from(&content[..])?;

		Ok(channel)
	}
}
