use crate::pipeline::cache::Cache;
use crate::pipeline::filter::{Field, Filter, Kind};
use crate::pipeline::retrieve::Retrieve;
use async_trait::async_trait;

use scraper::Selector;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt::Debug;
use std::time::Duration;

pub mod cache;
pub mod feed;
pub mod filter;
pub mod retrieve;

#[cfg(feature = "wasm")]
pub mod wasm;

#[async_trait]
pub trait Node: Sync + Send {
	type Item;

	async fn run(&self) -> anyhow::Result<Self::Item>;

	fn filter(self, field: Field, filter: Kind, invert: bool) -> Filter<Self>
	where
		Self: Sized,
	{
		Filter::new(self, field, filter, invert)
	}

	fn retrieve(self, content: Selector) -> Retrieve<Self>
	where
		Self: Serialize + DeserializeOwned + Debug,
	{
		Retrieve::new(self, content)
	}

	fn cache(self, ttl: Duration) -> Cache<Self>
	where
		Self: Serialize + DeserializeOwned + Debug,
	{
		Cache::new(self, ttl)
	}
}

#[async_trait]
impl<T> Node for Box<dyn Node<Item = T> + '_> {
	type Item = T;

	async fn run(&self) -> anyhow::Result<T> {
		(**self).run().await
	}
}

#[cfg(test)]
mod test {
	use crate::pipeline::feed::Feed;
	use crate::pipeline::filter::{Field, Kind};
	use crate::pipeline::Node;
	use ron::ser::PrettyConfig;
	use std::time::Duration;
	use tokio::time::sleep;

	#[tokio::test]
	pub async fn test() -> anyhow::Result<()> {
		tracing_subscriber::fmt::init();

		let pipe = Feed::new("https://www.azaleaellis.com/tag/pgts/feed".parse()?)
			.filter(
				Field::Description,
				Kind::Contains("BELOW IS A SNEAK PEEK OF THIS CONTENT!".parse()?),
				true,
			)
			.cache(Duration::from_secs(60 * 60));

		tracing::debug!(
			"{}",
			ron::ser::to_string_pretty(&pipe, PrettyConfig::default())?
		);

		let channel = &pipe.run().await?;
		tracing::info!("{}", channel.to_string());

		let channel = &pipe.run().await?;
		tracing::info!("{}", channel.to_string());

		sleep(Duration::from_secs(11)).await;
		let channel = &pipe.run().await?;
		tracing::info!("{}", channel.to_string());

		Ok(())
	}
}
