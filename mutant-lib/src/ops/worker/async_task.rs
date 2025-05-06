use std::fmt::Debug;

#[async_trait::async_trait]
pub trait AsyncTask<Item, Context, Client, TaskResult, TaskError>: Send + Sync + 'static
where
    Item: Send + 'static,
    Context: Send + Sync + 'static,
    Client: Send + Sync + 'static,
    TaskResult: Send + 'static,
    TaskError: Debug + Send + 'static,
{
    type ItemId: Send + Debug + Clone;

    async fn process(
        &self,
        worker_id: usize,
        client: &Client,
        item: Item,
    ) -> Result<(Self::ItemId, TaskResult), (TaskError, Item)>;
}
