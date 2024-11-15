use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Serialize, Deserialize, Clone, Debug, sqlx::FromRow)]
pub struct Code {
    pub id: String,
    pub owner_id: String,
    pub content: String,
    pub display_name: String,
    pub icon_url: Option<String>,
    pub website_url: Option<String>,
}

#[bon::bon]
impl Code {
    pub async fn get(
        pool: &SqlitePool,
        id: String,
        owner_id: String,
    ) -> Result<Code, sqlx::error::Error> {
        sqlx::query_as!(
            Code,
            "SELECT * FROM codes WHERE id = ? AND owner_id = ?",
            id,
            owner_id
        )
        .fetch_one(pool)
        .await
    }

    pub async fn get_many(
        pool: &SqlitePool,
        owner_id: String,
    ) -> Result<Vec<Code>, sqlx::error::Error> {
        sqlx::query_as!(Code, "SELECT * FROM codes WHERE owner_id = ?", owner_id)
            .fetch_all(pool)
            .await
    }

    pub async fn insert(&self, pool: &SqlitePool) -> Result<(), sqlx::error::Error> {
        sqlx::query!(
			"INSERT INTO codes (id, owner_id, content, display_name, icon_url, website_url) VALUES ($1, $2, $3, $4, $5, $6)",
			self.id, self.owner_id, self.content, self.display_name, self.icon_url, self.website_url).execute(pool).await?;

        Ok(())
    }

    pub async fn delete(&self, pool: &SqlitePool) -> Result<(), sqlx::error::Error> {
        sqlx::query!("DELETE FROM codes WHERE id = $1", self.id)
            .execute(pool)
            .await?;

        Ok(())
    }

    #[builder]
    pub async fn edit(
        &mut self,
        pool: &SqlitePool,
        content: Option<String>,
        display_name: Option<String>,
        icon_url: Option<String>,
        website_url: Option<String>,
    ) -> Result<&Code, sqlx::error::Error> {
        let mut tx = pool.begin().await?;

        if let Some(content_inner) = content {
            sqlx::query!(
                "UPDATE codes SET content = $2 WHERE id = $1",
                self.id,
                content_inner
            )
            .execute(&mut *tx)
            .await?;

            self.content = content_inner;
        }

        if let Some(display_name_inner) = display_name {
            sqlx::query!(
                "UPDATE codes SET display_name = $2 WHERE id = $1",
                self.id,
                display_name_inner
            )
            .execute(&mut *tx)
            .await?;

            self.display_name = display_name_inner;
        }

        if let Some(icon_url_inner) = icon_url {
            sqlx::query!(
                "UPDATE codes SET icon_url = $2 WHERE id = $1",
                self.id,
                icon_url_inner
            )
            .execute(&mut *tx)
            .await?;

            self.icon_url = Some(icon_url_inner);
        }

        if let Some(website_url_inner) = website_url {
            sqlx::query!(
                "UPDATE codes SET website_url = $2 WHERE id = $1",
                self.id,
                website_url_inner
            )
            .execute(&mut *tx)
            .await?;

            self.website_url = Some(website_url_inner);
        };

        tx.commit().await?;
        Ok(self)
    }
}
