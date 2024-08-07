use std::{cmp::min, slice, sync::Arc};

use anyhow::anyhow;
use async_trait::async_trait;
use atom_syndication::ContentBuilder;
use futures::stream::{self, StreamExt};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use super::node::{Data, DataKind, NodeTrait, IO};

/// Retrieves the full content of stub/summary entries.
#[derive(Serialize, Deserialize, Debug)]
pub struct Retrieve {
	#[serde(with = "serde_selector")]
	content: Selector,

	#[serde(skip)]
	input: Arc<IO>,
	#[serde(skip)]
	output: Arc<IO>,
}

impl Retrieve {
	pub fn new(content: Selector) -> Self {
		Self {
			content,
			input: Arc::default(),
			output: Arc::default(),
		}
	}
}

async fn get_content(
	mut entry: atom_syndication::Entry,
	selector: &Selector,
) -> anyhow::Result<atom_syndication::Entry> {
	let Some(link) = entry.links().iter().find(|l| l.rel().eq("alternate")) else {
		return Ok(entry);
	};

	tracing::info!("HTTP GET {}", link.href());
	let content = reqwest::get(link.href()).await?.text().await?;
	let html = Html::parse_document(&content);
	let content: String = html.select(selector).map(|s| s.inner_html()).collect();

	// item.description = None;
	entry.set_content(
		ContentBuilder::default()
			.value(content)
			.content_type("html".to_string())
			.build(),
	);

	Ok(entry)
}

#[async_trait]
impl NodeTrait for Retrieve {
	fn inputs(&self) -> &[Arc<IO>] {
		slice::from_ref(&self.input)
	}

	fn outputs(&self) -> &[Arc<IO>] {
		slice::from_ref(&self.output)
	}

	fn input_types(&self) -> &[DataKind] {
		&[DataKind::Feed]
	}

	fn output_types(&self) -> &[DataKind] {
		&[DataKind::Feed]
	}

	#[tracing::instrument(name = "retrieve_node", skip(self))]
	async fn run(&self) -> anyhow::Result<()> {
		let Some(Data::Feed(mut atom)) = self.input.get() else {
			return Err(anyhow!(""));
		};

		let n = min(atom.entries.len(), 6); // Avoiding too high values to prevent spamming the target site.
		let items: Vec<anyhow::Result<atom_syndication::Entry>> =
			stream::iter(atom.entries.into_iter())
				.map(|item| get_content(item, &self.content))
				.buffered(n)
				.collect()
				.await;
		atom.entries = items.into_iter().collect::<anyhow::Result<_>>()?;

		self.output.accept(atom)
	}

	fn set_input(&mut self, _index: usize, input: Arc<IO>) {
		self.input = input;
	}
	fn set_output(&mut self, _index: usize, output: Arc<IO>) {
		self.output = output;
	}
}

pub(crate) mod serde_selector {
	use scraper::{selector::ToCss, Selector};
	use serde::{Deserialize, Deserializer, Serializer};

	pub fn serialize<S>(selector: &Selector, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.serialize_str(&selector.to_css_string())
	}

	pub fn deserialize<'de, D>(deserializer: D) -> Result<Selector, D::Error>
	where
		D: Deserializer<'de>,
	{
		let s = String::deserialize(deserializer)?;
		Selector::parse(&s).map_err(serde::de::Error::custom)
	}
}
