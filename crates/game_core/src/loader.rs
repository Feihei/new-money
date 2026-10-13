//! 把 JSON 文件读成类型化资产。
//!
//! 不引第三方：Bevy 0.20 的 `Loaders::find` 先按资产类型再按扩展名选 loader，
//! 所以同一个 `.json` 扩展名可以注册多个 `JsonAssetLoader<A>`，`asset_server.load::<A>()`
//! 会路由到与 `A` 匹配的那个。

use bevy::asset::{Asset, AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;
use serde::de::DeserializeOwned;
use std::error::Error;
use std::marker::PhantomData;

#[derive(TypePath)]
pub struct JsonAssetLoader<A: Asset>(PhantomData<A>);

// 手写而非 derive：derive 会给 Default 加上 `A: Default` 这种无意义的约束。
impl<A: Asset> Default for JsonAssetLoader<A> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<A: Asset + DeserializeOwned> AssetLoader for JsonAssetLoader<A> {
    type Asset = A;
    type Settings = ();
    type Error = Box<dyn Error + Send + Sync>;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<A, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &["json"]
    }
}
