use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

pub mod feed;
#[cfg(feature = "filter")]
pub mod filter;
pub mod node;
#[cfg(feature = "retrieve")]
pub mod retrieve;
#[cfg(feature = "sanitise")]
pub mod sanitise;
#[cfg(feature = "wasm")]
pub mod wasm;

use node::{Data, DataKind, Node, NodeTrait, IO};

#[inline]
fn feed_io() -> Arc<IO> {
	Arc::new(IO::new(DataKind::Feed))
}

pub struct Flow {
	pub uuid: Uuid,
	nodes: Mutex<Vec<Node>>,

	output: Arc<IO>,
}

impl Flow {
	pub async fn run(&self) -> anyhow::Result<Option<Data>> {
		let nodes = self.nodes.lock().await;
		for node in nodes.iter() {
			if node.is_dirty() {
				tracing::info!("Running node: {node}");
				node.run().await?;

				let inputs = node.inputs();
				for io in inputs.iter().filter(|i| i.is_dirty()) {
					io.clear();
				}
			}
		}
		Ok(self.output.get())
	}
}

#[derive(Serialize, Deserialize)]
pub struct FlowBuilder {
	nodes: Vec<Node>,
}

impl FlowBuilder {
	pub fn new() -> Self {
		Self { nodes: Vec::new() }
	}

	pub fn node(mut self, node: impl Into<Node>) -> Self {
		self.nodes.push(node.into());
		self
	}

	pub fn simple(self, output: DataKind, uuid: Uuid) -> Flow {
		let mut nodes = self.nodes;
		let output = Arc::new(IO::new(output));

		let mut io = Some(output.clone());
		for node in nodes.iter_mut().rev() {
			if let Some(ioi) = io {
				node.output(ioi);
				io = None;
			}

			if let Some(input) = node.inputs().get(0) {
				io.replace(input.clone());
			}
		}

		Flow {
			uuid,
			nodes: Mutex::new(nodes),
			output,
		}
	}
}

impl From<Vec<Node>> for FlowBuilder {
	fn from(nodes: Vec<Node>) -> Self {
		FlowBuilder { nodes }
	}
}

#[cfg(test)]
mod test {
	use super::node::Field;
	use crate::flow::{
		feed::Feed,
		filter::{Filter, Kind},
		retrieve::Retrieve,
		sanitise::Sanitise,
		FlowBuilder,
	};
	use scraper::Selector;
	use std::time::Duration;

	#[tokio::test]
	pub async fn test() -> anyhow::Result<()> {
		let builder = FlowBuilder::new()
			.node(Feed::new(
				"https://www.azaleaellis.com/tag/pgts/feed/atom".parse()?,
				Duration::from_secs(60 * 60),
			))
			.node(Filter::new(
				Field::Summary,
				Kind::Contains("BELOW IS A SNEAK PEEK OF THIS CONTENT!".parse()?),
				true,
			))
			.node(Retrieve::new(Selector::parse(".entry-content").unwrap()))
			.node(Sanitise::new(Field::Content));

		println!("{}", serde_json::to_string_pretty(&builder)?);

		// let flow = builder.simple(DataKind::Feed);
		// let Some(Data::Feed(atom)) = flow.run().await? else {
		// 	return Err(anyhow!(""));
		// };
		//
		// println!("{}", atom.to_string());
		//
		// let Some(Data::Feed(atom)) = flow.run().await? else {
		// 	return Err(anyhow!(""));
		// };
		//
		// println!("Wow");

		//
		// let channel = &pipe.run().await?;
		// tracing::info!("{}", channel.to_string());
		//
		// let channel = &pipe.run().await?;
		// tracing::info!("{}", channel.to_string());
		//
		// sleep(Duration::from_secs(11)).await;
		// let channel = &pipe.run().await?;
		// tracing::info!("{}", channel.to_string());

		Ok(())
	}
}
