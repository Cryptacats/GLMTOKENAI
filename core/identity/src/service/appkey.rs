use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use chrono::Utc;
use futures::prelude::*;
use uuid::Uuid;
use ya_service_bus::{typed as bus, RpcEndpoint};

use ya_core_model::appkey as model;
use ya_core_model::identity as idm;
use ya_persistence::executor::DbExecutor;

use crate::dao::AppKeyDao;

#[derive(Default)]
struct Subscription {
    subscriptions: HashMap<u64, String>,
    last_id: u64,
}

impl Subscription {
    fn subscribe(&mut self, endpoint: String) -> u64 {
        let id = self.last_id;
        self.last_id += 1;
        let r = self.subscriptions.insert(id, endpoint);
        assert!(r.is_none());
        id
    }
}

fn send_events(s: Ref<Subscription>, event: model::event::Event) -> impl Future<Output = ()> {
    let destinations: Vec<String> = s.subscriptions.values().cloned().collect();

    // TODO: Remove on no destination.
    async move {
        for endpoint in destinations {
            match bus::service(&endpoint).call(event.clone()).await {
                Err(e) => log::error!("fail to send event: {}", e),
                Ok(Err(e)) => log::error!("fail to send event: {}", e),
                Ok(Ok(_)) => log::debug!("send event: {:?} to {}", event, endpoint),
            }
        }
    }
}

pub async fn activate(db: &DbExecutor) -> anyhow::Result<()> {
    let dbx = db.clone();
    let (tx, rx) = futures::channel::mpsc::unbounded();

    let subscription = Rc::new(RefCell::new(Subscription::default()));

    {
        let subscription = subscription.clone();
        tokio::task::spawn_local(async move {
            rx.for_each(|event| send_events(subscription.borrow(), event))
                .await;
        });
    }

    let _ = bus::bind(model::BUS_ID, move |s: model::Subscribe| {
        let id = subscription.borrow_mut().subscribe(s.endpoint);
        future::ok(id)
    });

    let create_tx = tx.clone();
    // Create a new application key entry
    let _ = bus::bind(model::BUS_ID, move |create: model::Create| {
        let key = Uuid::new_v4().to_simple().to_string();
        let db = dbx.clone();
        let mut create_tx = create_tx.clone();
        async move {
            let dao = db.as_dao::<AppKeyDao>();

            let result = match dao.get_for_name(create.name.clone()).await {
                Ok((app_key, _)) => {
                    if app_key.identity_id == create.identity {
                        Ok(app_key.key)
                    } else {
                        Err(model::Error::bad_request(format!(
                            "app-key with name {} already defined with identity {}",
                            app_key.name, app_key.identity_id
                        )))
                    }
                }
                Err(crate::dao::Error::Dao(diesel::result::Error::NotFound)) => dao
                    .create(
                        key.clone(),
                        create.name,
                        create.role,
                        create.identity,
                        create.allow_origins,
                    )
                    .await
                    .map_err(model::Error::internal)
                    .map(|_| key),
                Err(e) => Err(model::Error::internal(e)),
            }?;

            let (appkey, role) = db
                .as_dao::<AppKeyDao>()
                .get(result.clone())
                .await
                .map_err(|e| model::Error::internal(e.to_string()))?;

            let _ = create_tx
                .send(model::event::Event::NewKey(appkey.to_core_model(role)))
                .await;
            Ok(result)
        }
    });

    let dbx = db.clone();
    let preconfigured_appkey = crate::autoconf::preconfigured_appkey()?;
    let preconfigured_node_id = crate::autoconf::preconfigured_node_id()?;
    let start_datetime = Utc::now().naive_utc();
    let disable_appkey_security = std::env::var("YAGNA_DEV_DISABLE_APPKEY_SECURITY")
        .map(|f| f == "1")
        .unwrap_or(false);
    if disable_appkey_security {
        log::warn!("AppKey security is disabled. Not for production!");
    }
    // Retrieve an application key entry based on the key itself
    let _ = bus::bind(model::BUS_ID, move |get: model::Get| {
        let db = dbx.clone();
        let preconfigured_appkey = preconfigured_appkey.clone();
        async move {
            if preconfigured_appkey.as_ref() == Some(&get.key) {
                let node_id = match preconfigured_node_id {
                    Some(node_id) => node_id,
                    None => {
                        let default_identity = bus::service(idm::BUS_ID)
                            .send(idm::Get::ByDefault)
                            .await
                            .map_err(model::Error::internal)?
                            .map_err(model::Error::internal)?
                            .ok_or_else(|| model::Error::internal("appkey not found"))?;
                        default_identity.node_id
                    }
                };
                Ok(model::AppKey {
                    name: "autoconfigured".to_string(),
                    key: get.key.clone(),
                    role: model::DEFAULT_ROLE.to_string(),
                    identity: node_id,
                    created_date: start_datetime,
                    allow_origins: vec![],
                })
            } else {
                let (appkey, role) = match db
                    .as_dao::<AppKeyDao>()
                    .get(get.key.clone())
                    .await
                    .map_err(|e| model::Error::internal(e.to_string()))
                {
                    Ok((appkey, role)) => (appkey, role),
                    Err(e) => {
                        return if !disable_appkey_security {
                            //Normal path - return error
                            Err(e)
                        } else {
                            //Dev path - return default app-key, don't worry about unwraps
                            let node_id = match preconfigured_node_id {
                                Some(node_id) => node_id,
                                None => {
                                    let default_identity = bus::service(idm::BUS_ID)
                                        .send(idm::Get::ByDefault)
                                        .await
                                        .map_err(model::Error::internal)?
                                        .map_err(model::Error::internal)?
                                        .ok_or_else(|| {
                                            model::Error::internal("appkey not found")
                                        })?;
                                    default_identity.node_id
                                }
                            };
                            Ok(model::AppKey {
                                name: "autoconfigured".to_string(),
                                key: get.key.clone(),
                                role: model::DEFAULT_ROLE.to_string(),
                                identity: node_id,
                                created_date: start_datetime,
                            })
                        };
                    }
                };

                Ok(appkey.to_core_model(role))
            }
        }
    });

    let db_ = db.clone();
    let _ = bus::bind(model::BUS_ID, move |get: model::GetByName| {
        let db = db_.clone();
        async move {
            let (appkey, role) = db
                .as_dao::<AppKeyDao>()
                .get_for_name(get.name)
                .await
                .map_err(|e| model::Error::internal(e.to_string()))?;

            Ok(appkey.to_core_model(role))
        }
    });

    let dbx = db.clone();
    let _ = bus::bind(model::BUS_ID, move |list: model::List| {
        let db = dbx.clone();

        async move {
            let result = db
                .as_dao::<AppKeyDao>()
                .list(list.identity, list.page, list.per_page)
                .await
                .map_err(Into::<model::Error>::into)?;

            let keys = result
                .0
                .into_iter()
                .map(|(app_key, role)| app_key.to_core_model(role))
                .collect();

            Ok((keys, result.1))
        }
    });

    let create_tx = tx;
    let dbx = db.clone();
    let _ = bus::bind(model::BUS_ID, move |rm: model::Remove| {
        let db = dbx.clone();
        let mut create_tx = create_tx.clone();
        async move {
            let (appkey, role) = db
                .as_dao::<AppKeyDao>()
                .get_for_name(rm.name.clone())
                .await
                .map_err(|e| model::Error::internal(e.to_string()))?;

            db.as_dao::<AppKeyDao>()
                .remove(rm.name, rm.identity)
                .await
                .map_err(Into::<model::Error>::into)?;

            let _ = create_tx
                .send(model::event::Event::DroppedKey(appkey.to_core_model(role)))
                .await;
            Ok(())
        }
    });

    Ok(())
}
