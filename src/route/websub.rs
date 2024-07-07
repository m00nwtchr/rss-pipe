use anyhow::anyhow;
use axum::{
	body::Bytes,
	extract::{Path, Query, State},
	http::{HeaderMap, HeaderName, StatusCode},
	response::IntoResponse,
	routing::{get, post},
	Router,
};
use chrono::Utc;
use serde::Deserialize;
use sha2::{Sha256, Sha384, Sha512};
use sqlx::{Executor, Row, SqlitePool};
use std::{str::FromStr, time::Duration};
use uuid::Uuid;

use super::internal_error;
use crate::{
	app::AppState,
	flow::node::{DataKind, NodeTrait},
};

#[allow(clippy::declare_interior_mutable_const)]
const X_HUB_SIGNATURE: HeaderName = HeaderName::from_static("x-hub-signature");

pub async fn receive(
	Path(uuid): Path<Uuid>,
	State(pool): State<SqlitePool>,
	State(state): State<AppState>,
	headers: HeaderMap,
	body: Bytes,
) -> Result<impl IntoResponse, (StatusCode, String)> {
	let uuid = uuid.as_bytes().as_slice();

	let mut conn = pool.acquire().await.map_err(internal_error)?;
	if let Some(row) = conn
		.fetch_optional(sqlx::query!(
			"SELECT flow, secret FROM websub WHERE uuid = ?",
			uuid
		))
		.await
		.map_err(internal_error)?
	{
		let secret: &str = row.get("secret");
		let signature = headers.get(X_HUB_SIGNATURE);

		let Some(signature) = signature
			.and_then(|v| v.to_str().ok())
			.and_then(|s| XHubSignature::from_str(s).ok())
		else {
			return Err((StatusCode::FORBIDDEN, StatusCode::FORBIDDEN.to_string()));
		};

		let verified = signature
			.verify(secret.as_bytes(), &body)
			.map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;

		// let verified = if let Some(secret) = secret {
		// 	let Some(signature) = signature
		// 		.and_then(|v| v.to_str().ok())
		// 		.and_then(|s| XHubSignature::from_str(s).ok())
		// 	else {
		// 		return Err((StatusCode::FORBIDDEN, StatusCode::FORBIDDEN.to_string()));
		// 	};
		//
		// 	signature
		// 		.verify(secret, &body)
		// 		.map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?
		// } else {
		// 	signature.is_none()
		// };

		if verified {
			let flow_name: &str = row.get("flow");
			let Some(flow) = state.flows.lock().await.get(flow_name).cloned() else {
				return Err((StatusCode::NOT_FOUND, "Invalid subscription".to_string()));
			};

			if let Some(input) = flow
				.inputs()
				.iter()
				.find(|i| matches!(i.kind(), DataKind::WebSub))
			{
				tracing::info!("Received WebSub push for `{flow_name}`");
				input
					.accept(body)
					.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

				flow.run()
					.await
					.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
			}
		} else {
			return Err((StatusCode::FORBIDDEN, "Invalid signature".to_string()));
		}
	}

	Ok(StatusCode::OK)
}

#[derive(Deserialize, Debug)]
#[serde(tag = "hub.mode", rename_all = "lowercase")]
pub enum Verification {
	Subscribe {
		#[serde(rename = "hub.topic")]
		topic: String,
		#[serde(rename = "hub.challenge")]
		challenge: String,
		#[serde(rename = "hub.lease_seconds", deserialize_with = "de::deserialize")]
		lease_seconds: Duration,
	},
	Unsubscribe {
		#[serde(rename = "hub.topic")]
		topic: String,
		#[serde(rename = "hub.challenge")]
		challenge: String,
	},
}

pub async fn verify(
	Path(uuid): Path<Uuid>,
	State(pool): State<SqlitePool>,
	Query(verification): Query<Verification>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
	let uuid = uuid.as_bytes().as_slice();

	tracing::info!("{verification:?}");

	let mut conn = pool.acquire().await.map_err(internal_error)?;
	if let Some(row) = conn
		.fetch_optional(sqlx::query!(
			"SELECT subscribed, topic FROM websub WHERE uuid = ?",
			uuid
		))
		.await
		.map_err(internal_error)?
	{
		let subscribed = row.get("subscribed");
		let my_topic: &str = row.get("topic");

		match verification {
			Verification::Subscribe {
				topic,
				challenge,
				lease_seconds,
			} => {
				let lease_end = Utc::now() + lease_seconds;
				conn.execute(sqlx::query!(
					"UPDATE websub SET lease_end = ? WHERE uuid = ?",
					lease_end,
					uuid
				))
				.await
				.map_err(internal_error)?;

				if subscribed && topic.eq(my_topic) {
					Ok((StatusCode::OK, challenge))
				} else {
					Err((StatusCode::BAD_REQUEST, "Bad request".to_string()))
				}
			}
			Verification::Unsubscribe { topic, challenge } => {
				if !subscribed && topic.eq(my_topic) {
					conn.execute(sqlx::query!("DELETE FROM websub WHERE uuid = ?", uuid))
						.await
						.map_err(internal_error)?;
					Ok((StatusCode::OK, challenge))
				} else {
					Err((StatusCode::BAD_REQUEST, "Bad request".to_string()))
				}
			}
		}
	} else {
		Err((StatusCode::NOT_FOUND, "Not found".to_string()))
	}
}

pub fn router() -> Router<AppState> {
	Router::new()
		.route("/:uuid", post(receive))
		.route("/:uuid", get(verify))
}

mod de {
	use serde::{Deserialize, Deserializer};
	use std::time::Duration;

	pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
	where
		D: Deserializer<'de>,
	{
		let s = String::deserialize(deserializer)?
			.parse()
			.map_err(serde::de::Error::custom)?;
		Ok(Duration::from_secs(s))
	}
}

#[derive(Debug)]
pub struct XHubSignature {
	method: String,
	signature: Vec<u8>,
}

impl XHubSignature {
	#[tracing::instrument(skip(secret, message))]
	pub fn verify(&self, secret: &[u8], message: &[u8]) -> anyhow::Result<bool> {
		Ok(match self.method.as_str() {
			#[cfg(feature = "sha1")]
			"sha1" => mac::verify_hmac::<sha1::Sha1>(&self.signature, secret, message)?,
			#[cfg(not(feature = "sha1"))]
			"sha1" => {
				tracing::error!("Unsupported sha1 signature on WebSub push");
				false
			}
			"sha256" => mac::verify_hmac::<Sha256>(&self.signature, secret, message)?,
			"sha384" => mac::verify_hmac::<Sha384>(&self.signature, secret, message)?,
			"sha512" => mac::verify_hmac::<Sha512>(&self.signature, secret, message)?,
			_ => {
				tracing::error!("Unknown signature algorithm");
				false
			}
		})
	}
}

impl FromStr for XHubSignature {
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let Some((method, signature)) = s.split_once('=') else {
			return Err(anyhow!(""));
		};

		Ok(XHubSignature {
			method: method.to_string(),
			signature: hex::decode(signature)?,
		})
	}
}

mod mac {
	use hmac::{
		digest::{
			block_buffer::Eager,
			consts::U256,
			core_api::{BlockSizeUser, BufferKindUser, CoreProxy, FixedOutputCore, UpdateCore},
			typenum::{IsLess, Le, NonZero},
			HashMarker,
		},
		Hmac, Mac,
	};

	pub fn verify_hmac<D>(signature: &[u8], secret: &[u8], message: &[u8]) -> anyhow::Result<bool>
	where
		D: CoreProxy,
		D::Core: HashMarker
			+ UpdateCore
			+ FixedOutputCore
			+ BufferKindUser<BufferKind = Eager>
			+ Default
			+ Clone,
		<D::Core as BlockSizeUser>::BlockSize: IsLess<U256>,
		Le<<D::Core as BlockSizeUser>::BlockSize, U256>: NonZero,
	{
		let mut hmac: Hmac<D> = Hmac::new_from_slice(secret)?;
		hmac.update(message);
		Ok(hmac.verify_slice(signature).is_ok())
	}
}
