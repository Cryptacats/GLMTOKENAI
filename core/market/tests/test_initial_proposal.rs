use ya_client::model::market::event::{ProviderEvent, RequestorEvent};
use ya_client::model::market::proposal::State;
use ya_market::testing::events_helper::ClientProposalHelper;
use ya_market::testing::mock_offer::client::{sample_demand, sample_offer};
use ya_market::testing::{MarketServiceExt, MarketsNetwork, OwnerType};
use ya_market::testing::{QueryEventsError, TakeEventsError};
use ya_market::MarketService;

use std::sync::Arc;
use std::time::Duration;

/// No events available for not existent subscription.
#[cfg_attr(not(feature = "test-suite"), ignore)]
#[actix_rt::test]
#[serial_test::serial]
async fn test_query_events_non_existent_subscription() -> Result<(), anyhow::Error> {
    let network = MarketsNetwork::new(None)
        .await
        .add_market_instance("Node-1")
        .await?;

    let market1 = network.get_market("Node-1");
    let non_existent_id = "80da375cb604426fb6cddd64f4ccc715-85fdde1924371f4a3a412748f61e5b941c500ea69a55a5135b886a2bffcb8e55".parse()?;

    // We expect that no events are available for non existent subscription.
    let result = market1.query_events(&non_existent_id, 1.2, Some(5)).await;

    assert_eq!(
        result.unwrap_err().to_string(),
        TakeEventsError::SubscriptionNotFound(non_existent_id).to_string()
    );

    Ok(())
}

/// Initial proposal generated by market should be available at
/// query events endpoint.
#[cfg_attr(not(feature = "test-suite"), ignore)]
#[actix_rt::test]
#[serial_test::serial]
async fn test_query_initial_proposal() -> Result<(), anyhow::Error> {
    let network = MarketsNetwork::new(None)
        .await
        .add_market_instance("Node-1")
        .await?;

    let market1 = network.get_market("Node-1");
    let identity1 = network.get_default_id("Node-1");
    let identity2 = network.create_identity("Node-1", "Identity2");

    let demand_id = market1
        .subscribe_demand(&sample_demand(), &identity1)
        .await?;
    market1.subscribe_offer(&sample_offer(), &identity2).await?;

    // We expect that proposal will be available as requestor event.
    let events = market1.query_events(&demand_id, 1.0, Some(5)).await?;

    assert_eq!(events.len(), 1);

    let proposal = match events[0].clone() {
        RequestorEvent::ProposalEvent { proposal, .. } => proposal,
        _ => panic!("Invalid event Type. ProposalEvent expected"),
    };

    assert_eq!(proposal.prev_proposal_id, None);
    assert_eq!(proposal.state, State::Initial);

    // We expect that, the same event won't be available again.
    let events = market1.query_events(&demand_id, 0.2, Some(5)).await?;

    assert_eq!(events.len(), 0);

    Ok(())
}

/// Test getting more then one event from query.
#[cfg_attr(not(feature = "test-suite"), ignore)]
#[actix_rt::test]
#[serial_test::serial]
async fn test_query_multiple_events() -> Result<(), anyhow::Error> {
    let network = MarketsNetwork::new(None)
        .await
        .add_market_instance("Node-1")
        .await?;

    let market1 = network.get_market("Node-1");
    let identity1 = network.get_default_id("Node-1");
    let identity2 = network.create_identity("Node-1", "Identity2");

    let demand_id = market1
        .subscribe_demand(&sample_demand(), &identity2)
        .await?;
    let offer = sample_offer();
    market1.subscribe_offer(&offer, &identity1).await?;
    market1.subscribe_offer(&offer, &identity1).await?;
    market1.subscribe_offer(&offer, &identity1).await?;

    // We expect that 3 proposal will be available as requestor event.
    let mut events = vec![];
    for _ in 0..3 {
        events.append(&mut market1.query_events(&demand_id, 1.0, Some(5)).await?);
    }
    assert_eq!(events.len(), 3);

    for event in events {
        match event {
            RequestorEvent::ProposalEvent { proposal, .. } => {
                assert_eq!(proposal.prev_proposal_id, None);
                assert_eq!(proposal.state, State::Initial);
            }
            _ => panic!("ProposalEvent expected, but got {:?}", event),
        };
    }

    // We expect that, the same events won't be available again.
    let events = market1.query_events(&demand_id, 0.2, Some(5)).await?;

    assert_eq!(events.len(), 0);

    Ok(())
}

/// Query_events should hang on endpoint until event will come
/// or timeout elapses.
#[cfg_attr(not(feature = "test-suite"), ignore)]
#[actix_rt::test]
#[serial_test::serial]
async fn test_query_events_timeout() -> Result<(), anyhow::Error> {
    let network = MarketsNetwork::new(None)
        .await
        .add_market_instance("Node-1")
        .await?;

    let market1 = network.get_market("Node-1");
    let identity1 = network.get_default_id("Node-1");
    let identity2 = network.create_identity("Node-1", "Identity2");

    let demand_id1 = market1
        .subscribe_demand(&sample_demand(), &identity1)
        .await?;
    let demand_id1c = demand_id1.clone();
    let market1c = market1.clone();

    // Query events, when no Proposal are in the queue yet.
    // We set timeout and we expect that function will wait until events will come.
    let query_handle = tokio::spawn(async move {
        let events = market1c.query_events(&demand_id1c, 1.2, Some(5)).await?;
        assert_eq!(events.len(), 1);
        Result::<(), anyhow::Error>::Ok(())
    });

    // Inject proposal before timeout will elapse. We expect that Proposal
    // event will be generated and query events will return it.
    tokio::time::delay_for(Duration::from_millis(50)).await;
    market1.subscribe_offer(&sample_offer(), &identity2).await?;

    // Protect from eternal waiting.
    tokio::time::timeout(Duration::from_millis(1250), query_handle).await???;
    Ok(())
}

/// Query events will return before timeout will elapse, if Demand will be unsubscribed.
#[cfg_attr(not(feature = "test-suite"), ignore)]
#[actix_rt::test]
#[serial_test::serial]
async fn test_query_events_unsubscribe_notification() -> Result<(), anyhow::Error> {
    let network = MarketsNetwork::new(None)
        .await
        .add_market_instance("Node-1")
        .await?;

    let market1 = network.get_market("Node-1");
    let identity1 = network.get_default_id("Node-1");

    let subscription_id = market1
        .subscribe_demand(&sample_demand(), &identity1)
        .await?;
    let demand_id = subscription_id.clone();

    // Query events, when no Proposal are in the queue yet.
    // We set timeout and we expect that function will wait until events will come.
    let query_handle = tokio::spawn(async move {
        match market1.query_events(&subscription_id, 1.2, Some(5)).await {
            Err(QueryEventsError::Unsubscribed(id)) => {
                assert_eq!(id, subscription_id);
            }
            x => panic!("Expected Unsubscribed error, but got {:?}", x),
        }
        Result::<(), anyhow::Error>::Ok(())
    });

    // Unsubscribe Demand. query_events should return with unsubscribed error.
    tokio::time::delay_for(Duration::from_millis(50)).await;

    let market1 = network.get_market("Node-1");
    market1.unsubscribe_demand(&demand_id, &identity1).await?;

    // Protect from eternal waiting.
    tokio::time::timeout(Duration::from_millis(1250), query_handle).await???;

    Ok(())
}

/// Tests if query events returns proper error on invalid input
/// or unsubscribed demand.
#[cfg_attr(not(feature = "test-suite"), ignore)]
#[actix_rt::test]
#[serial_test::serial]
async fn test_query_events_edge_cases() -> Result<(), anyhow::Error> {
    let network = MarketsNetwork::new(None)
        .await
        .add_market_instance("Node-1")
        .await?;

    let market1 = network.get_market("Node-1");
    let identity1 = network.get_default_id("Node-1");
    let identity2 = network.create_identity("Node-1", "Identity2");

    let demand_id = market1
        .subscribe_demand(&sample_demand(), &identity2)
        .await?;
    market1.subscribe_offer(&sample_offer(), &identity1).await?;

    // We should reject calls with negative maxEvents.
    let result = market1.query_events(&demand_id, 0.0, Some(-5)).await;
    assert_eq!(
        result.unwrap_err().to_string(),
        QueryEventsError::InvalidMaxEvents(-5, 100).to_string()
    );

    // Negative timeout should be treated as immediate checking events and return.
    tokio::time::delay_for(Duration::from_millis(200)).await;
    let events = tokio::time::timeout(
        Duration::from_millis(20),
        market1
            .requestor_engine
            .query_events(&demand_id, -5.0, None),
    )
    .await??;
    assert_eq!(events.len(), 1);

    // Restore available Proposal
    let demand_id = market1
        .subscribe_demand(&sample_demand(), &identity2)
        .await?;
    market1.subscribe_offer(&sample_offer(), &identity1).await?;

    // maxEvents equal to 0 is forbidden value now.
    let result = market1.query_events(&demand_id, 0.2, Some(0)).await;
    assert_eq!(
        result.unwrap_err().to_string(),
        QueryEventsError::InvalidMaxEvents(0, 100).to_string()
    );

    // Query events returns error, if Demand was unsubscribed.
    market1.unsubscribe_demand(&demand_id, &identity2).await?;

    let result = market1.query_events(&demand_id, 0.0, None).await;
    assert_eq!(
        result.unwrap_err().to_string(),
        TakeEventsError::SubscriptionNotFound(demand_id).to_string()
    );

    Ok(())
}

/// Generate proposals for multiple subscriptions. Query events should return
/// only events related for requested subscription and shouldn't affect remaining events.
#[cfg_attr(not(feature = "test-suite"), ignore)]
#[actix_rt::test]
#[serial_test::serial]
async fn test_query_events_for_multiple_subscriptions() -> Result<(), anyhow::Error> {
    let network = MarketsNetwork::new(None)
        .await
        .add_market_instance("Node-1")
        .await?;

    let market1 = network.get_market("Node-1");
    let identity1 = network.get_default_id("Node-1");
    let identity2 = network.create_identity("Node-1", "Identity2");

    // Spawn 3 Demands and 1 Offer --> should result in 3 Proposals.
    let demand_id1 = market1
        .subscribe_demand(&sample_demand(), &identity1)
        .await?;
    market1.subscribe_offer(&sample_offer(), &identity2).await?;
    let demand_id2 = market1
        .subscribe_demand(&sample_demand(), &identity1)
        .await?;
    let demand_id3 = market1
        .subscribe_demand(&sample_demand(), &identity1)
        .await?;

    // Check events related to first and last subscription.
    let events = market1.query_events(&demand_id1, 1.2, Some(5)).await?;
    assert_eq!(events.len(), 1);

    // Unsubscribe subscription 3. Events on subscription 2 should be still available.
    market1.unsubscribe_demand(&demand_id3, &identity1).await?;

    let events = market1.query_events(&demand_id2, 1.2, Some(5)).await?;
    assert_eq!(events.len(), 1);
    Ok(())
}

/// Run two query events in the same time.
/// The same event shouldn't be returned twice.
#[cfg_attr(not(feature = "test-suite"), ignore)]
#[actix_rt::test]
#[serial_test::serial]
async fn test_simultaneous_query_events() -> Result<(), anyhow::Error> {
    let network = MarketsNetwork::new(None)
        .await
        .add_market_instance("Node-1")
        .await?;

    let market1 = network.get_market("Node-1");
    let identity1 = network.get_default_id("Node-1");
    let identity2 = network.create_identity("Node-1", "Identity2");

    let demand_id1 = market1
        .subscribe_demand(&sample_demand(), &identity2)
        .await?;

    let demand_id = demand_id1.clone();
    let market = market1.clone();

    let query1 = tokio::spawn(async move {
        let events = market.query_events(&demand_id, 1.2, Some(5)).await?;
        Result::<_, anyhow::Error>::Ok(events)
    });

    let market = market1.clone();
    let demand_id = demand_id1.clone();

    let query2 = tokio::spawn(async move {
        let events = market.query_events(&demand_id, 1.2, Some(5)).await?;
        Result::<_, anyhow::Error>::Ok(events)
    });

    // Wait for a while, before event will be injected. We want to trigger notifications.
    // Generate 2 proposals. Each waiting query events call will take an event.
    tokio::time::delay_for(Duration::from_millis(50)).await;
    market1.subscribe_offer(&sample_offer(), &identity1).await?;
    market1.subscribe_offer(&sample_offer(), &identity1).await?;

    let mut events1 = tokio::time::timeout(Duration::from_millis(1250), query1).await???;
    let events2 = tokio::time::timeout(Duration::from_millis(1250), query2).await???;

    // We expect no events duplication.
    assert_eq!(events1.len() + events2.len(), 2);
    events1.extend(events2.iter().cloned());

    let ids = events1
        .into_iter()
        .map(|event| match event {
            RequestorEvent::ProposalEvent { proposal, .. } => proposal.proposal_id,
            _ => panic!("Expected ProposalEvents"),
        })
        .collect::<Vec<String>>();
    assert_ne!(ids[0], ids[1]);

    // We expect, there are no events left.
    let events = market1.query_events(&demand_id1, 0.1, Some(5)).await?;
    assert_eq!(events.len(), 0);
    Ok(())
}

/// Run two query events in the same time.
/// The same event shouldn't be returned twice.
#[cfg_attr(not(feature = "test-suite"), ignore)]
#[actix_rt::test]
#[serial_test::serial]
async fn test_unsubscribe_demand_while_query_events_for_other() -> Result<(), anyhow::Error> {
    let network = MarketsNetwork::new(None)
        .await
        .add_market_instance("Node-1")
        .await?;

    let market1 = network.get_market("Node-1");
    let identity1 = network.get_default_id("Node-1");
    let identity2 = network.create_identity("Node-1", "Identity2");

    let demand_id1 = market1
        .subscribe_demand(&sample_demand(), &identity1)
        .await?;
    let demand_id2 = market1
        .subscribe_demand(&sample_demand(), &identity1)
        .await?;

    let demand_id1_copy = demand_id1.clone();
    let market_copy = market1.clone();

    let query = tokio::spawn(async move {
        let events = market_copy
            .query_events(&demand_id1_copy, 1.2, Some(5))
            .await?;
        Result::<_, anyhow::Error>::Ok(events)
    });

    // Wait for a while, and unsubscribe second demand and subscribe first offer.
    tokio::time::delay_for(Duration::from_millis(50)).await;
    market1.unsubscribe_demand(&demand_id2, &identity1).await?;
    market1.subscribe_offer(&sample_offer(), &identity2).await?;

    // Query events for first demand should return single Proposal.
    let events = tokio::time::timeout(Duration::from_millis(1250), query).await???;
    assert_eq!(events.len(), 1);

    // We expect, there are no events left.
    let events = market1.query_events(&demand_id1, 0.1, Some(5)).await?;
    assert_eq!(events.len(), 0);
    Ok(())
}

/// Requestor gets initial proposal from market and counters it. Proposal should be
/// propagated to Provider node and added to events.
/// Both Offer and Demand are on the same node.
#[cfg_attr(not(feature = "test-suite"), ignore)]
#[actix_rt::test]
#[serial_test::serial]
async fn test_counter_initial_proposal() -> Result<(), anyhow::Error> {
    let network = MarketsNetwork::new(None)
        .await
        .add_market_instance("Node-1")
        .await?;

    let market1: Arc<MarketService> = network.get_market("Node-1");
    let identity1 = network.get_default_id("Node-1");
    let identity2 = network.create_identity("Node-1", "Identity2");

    let subscription_id = market1
        .subscribe_demand(&sample_demand(), &identity1)
        .await?;
    let offer_id = market1.subscribe_offer(&sample_offer(), &identity2).await?;

    // We expect that proposal will be available as event.
    let events = market1
        .requestor_engine
        .query_events(&subscription_id, 1.0, Some(5))
        .await?;

    assert_eq!(events.len(), 1);
    let init_proposal = match events[0].clone() {
        RequestorEvent::ProposalEvent { proposal, .. } => proposal,
        _ => panic!("Invalid event Type. ProposalEvent expected"),
    };
    let init_proposal_id = init_proposal.get_proposal_id()?;

    let counter_proposal = sample_demand();
    let new_proposal_id = market1
        .requestor_engine
        .counter_proposal(
            &subscription_id,
            &init_proposal_id,
            &counter_proposal,
            &identity1,
        )
        .await?;
    assert_ne!(&new_proposal_id, &init_proposal_id);

    // We expect that event was generated on Provider part of Node.
    let new_proposal_id = new_proposal_id.translate(OwnerType::Provider);
    let events = market1
        .provider_engine
        .query_events(&offer_id, 1.5, Some(5))
        .await?;
    assert_eq!(events.len(), 1);

    let proposal = match events[0].clone() {
        ProviderEvent::ProposalEvent { proposal, .. } => proposal,
        _ => panic!("Invalid event Type. ProposalEvent expected"),
    };
    assert_eq!(proposal.issuer_id, identity1.identity);
    assert_eq!(proposal.proposal_id, new_proposal_id.to_string());
    assert_eq!(proposal.state, State::Draft);

    // Provider creates internally Proposal corresponding to initial Proposal
    // on Requestor, but id will be different.
    assert!(proposal.prev_proposal_id.is_some());
    Ok(())
}
