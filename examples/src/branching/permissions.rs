use iota_streams::{
    app::{
        message::HasLink,
        permission::*,
    },
    app_channels::api::{
        psk_from_seed,
        pskid_from_psk,
        tangle::{
            Author,
            ChannelType,
            Subscriber,
            Transport,
        },
    },
    core::{
        assert,
        prelude::HashMap,
        print,
        println,
        try_or,
        Errors::*,
        Result,
    },
    ddml::types::*,
};

use super::utils;

pub async fn example<T: Transport>(transport: T, channel_impl: ChannelType, seed: &str) -> Result<()> {
    let mut author = Author::new(seed, channel_impl, transport.clone()).await;
    println!("Author multi branching?: {}", author.is_multi_branching());

    let mut subscriberA = Subscriber::new("SUBSCRIBERA9SEED", transport.clone()).await;
    let mut subscriberB = Subscriber::new("SUBSCRIBERB9SEED", transport.clone()).await;
    let mut subscriberC = Subscriber::new("SUBSCRIBERC9SEED", transport).await;

    let subA_xkey = subscriberA.key_exchange_public_key()?;

    let public_payload = Bytes("PUBLICPAYLOAD".as_bytes().to_vec());
    let masked_payload = Bytes("MASKEDPAYLOAD".as_bytes().to_vec());

    println!("\nAnnounce Channel");
    let announcement_link = {
        let msg = author.send_announce().await?;
        println!("  msg => <{}> <{:x}>", msg.msgid, msg.to_msg_index());
        print!("  Author     : {}", author);
        msg
    };
    println!("  Author channel address: {}", author.channel_address().unwrap());

    println!("\nHandle Announce Channel");
    {
        subscriberA.receive_announcement(&announcement_link).await?;
        print!("  SubscriberA: {}", subscriberA);
        try_or!(
            author.channel_address() == subscriberA.channel_address(),
            ApplicationInstanceMismatch(String::from("A"))
        )?;
        subscriberB.receive_announcement(&announcement_link).await?;
        print!("  SubscriberB: {}", subscriberB);
        try_or!(
            author.channel_address() == subscriberB.channel_address(),
            ApplicationInstanceMismatch(String::from("B"))
        )?;
        subscriberC.receive_announcement(&announcement_link).await?;
        print!("  SubscriberC: {}", subscriberC);
        try_or!(
            author.channel_address() == subscriberC.channel_address(),
            ApplicationInstanceMismatch(String::from("C"))
        )?;

        try_or!(
            subscriberA
                .channel_address()
                .map_or(false, |appinst| appinst == announcement_link.base()),
            ApplicationInstanceMismatch(String::from("A"))
        )?;
        try_or!(
            subscriberA.is_multi_branching() == author.is_multi_branching(),
            BranchingFlagMismatch(String::from("A"))
        )?;
    }

    // Predefine Subscriber A
    println!("\nAuthor Predefines Subscriber A");
    author.store_new_subscriber(*subscriberA.id(), subA_xkey)?;

    // Generate a simple PSK for storage by users
    let psk = psk_from_seed("A pre shared key".as_bytes());
    let pskid = pskid_from_psk(&psk);
    author.store_psk(pskid, psk)?;
    subscriberC.store_psk(pskid, psk)?;

    // Fetch state of subscriber for comparison after reset
    let sub_a_start_state: HashMap<_, _> = subscriberA.fetch_state()?.into_iter().collect();

    println!("\nSubscribe B");
    let subscribeB_link = {
        let msg = subscriberB.send_subscribe(&announcement_link).await?;
        println!("  msg => <{}> <{:x}>", msg.msgid, msg.to_msg_index());
        print!("  SubscriberB: {}", subscriberB);
        msg
    };

    println!("\nHandle Subscribe B");
    {
        author.receive_subscribe(&subscribeB_link).await?;
        print!("  Author     : {}", author);
    }

    let sub_a_perm = Permission::ReadWrite(subscriberA.id().clone(), PermissionDuration::Perpetual);
    let sub_b_perm = Permission::Read(subscriberB.id().clone());
    let psk_perm = Permission::Read(pskid.into());
    let permissions = vec![sub_a_perm, sub_b_perm, psk_perm];

    println!("\nShare keyload for subscribers [SubscriberA, SubscriberB, PSK]");
    let previous_msg_link = {
        let (msg, seq) = author.send_keyload(&announcement_link, &permissions).await?;
        println!("  msg => <{}> <{:x}>", msg.msgid, msg.to_msg_index());
        assert!(seq.is_none());
        print!("  Author     : {}", author);
        msg
    };

    println!("\nHandle Keyload");
    {
        subscriberA.receive_keyload(&previous_msg_link).await?;
        print!("  SubscriberA: {}", subscriberA);
        subscriberB.receive_keyload(&previous_msg_link).await?;
        print!("  SubscriberB: {}", subscriberB);
        subscriberC.receive_keyload(&previous_msg_link).await?;
        print!("  SubscriberC: {}", subscriberC);
    }

    println!("\nSigned packet");
    let previous_msg_link = {
        let (msg, seq) = author
            .send_signed_packet(&previous_msg_link, &public_payload, &masked_payload)
            .await?;
        println!("  msg => <{}> <{:x}>", msg.msgid, msg.to_msg_index());
        assert!(seq.is_none());
        print!("  Author     : {}", author);
        msg
    };

    println!("\nHandle Signed packet");
    {
        let (_signer_pk, unwrapped_public, unwrapped_masked) =
            subscriberA.receive_signed_packet(&previous_msg_link).await?;
        print!("  SubscriberA: {}", subscriberA);
        try_or!(
            public_payload == unwrapped_public,
            PublicPayloadMismatch(public_payload.to_string(), unwrapped_public.to_string())
        )?;
        try_or!(
            masked_payload == unwrapped_masked,
            PublicPayloadMismatch(masked_payload.to_string(), unwrapped_masked.to_string())
        )?;
    }
    
    println!("\nSubscriber B fetching transactions...");
    utils::fetch_next_messages(&mut subscriberB).await?;
    println!("\nSubscriber C fetching transactions...");
    utils::fetch_next_messages(&mut subscriberC).await?;

    println!("\nTagged packet 1 - SubscriberB (tagged is always allowed)");
    let previous_msg_link = {
        let (msg, seq) = subscriberB
            .send_tagged_packet(&previous_msg_link, &public_payload, &masked_payload)
            .await?;
        println!("  msg => <{}> <{:x}>", msg.msgid, msg.to_msg_index());
        assert!(seq.is_none());
        print!("  SubscriberB: {}", subscriberB);
        msg
    };

    println!("\nHandle Tagged packet 1");
    {
        let (unwrapped_public, unwrapped_masked) = author.receive_tagged_packet(&previous_msg_link).await?;
        print!("  Author     : {}", author);
        try_or!(
            public_payload == unwrapped_public,
            PublicPayloadMismatch(public_payload.to_string(), unwrapped_public.to_string())
        )?;
        try_or!(
            masked_payload == unwrapped_masked,
            PublicPayloadMismatch(masked_payload.to_string(), unwrapped_masked.to_string())
        )?;
    }
    
    println!("\nSubscriber A fetching transactions...");
    utils::fetch_next_messages(&mut subscriberA).await?;

    println!("\nSigned packet 1 - SubscriberA (subscriber A has Write permission");
    let previous_msg_link = {
        let (msg, seq) = subscriberA
            .send_signed_packet(&previous_msg_link, &public_payload, &masked_payload)
            .await?;
        println!("  msg => <{}> <{:x}>", msg.msgid, msg.to_msg_index());
        assert!(seq.is_none());
        print!("  SubscriberA: {}", subscriberA);
        msg
    };

    println!("\nAuthor fetching transactions...");
    utils::fetch_next_messages(&mut author).await?;
    println!("\nSubscriber B fetching transactions...");
    utils::fetch_next_messages(&mut subscriberB).await?;
    println!("\nSubscriber C fetching transactions...");
    utils::fetch_next_messages(&mut subscriberC).await?;

    println!("\nSigned packet 1 - SubscriberB (subscriber B does NOT has Write permission");
    let previous_msg_link = {
        let (msg, seq) = subscriberB
            .send_signed_packet(&previous_msg_link, &public_payload, &masked_payload)
            .await?;
        println!("  msg => <{}> <{:x}>", msg.msgid, msg.to_msg_index());
        assert!(seq.is_none());
        print!("  SubscriberB: {}", subscriberB);
        msg
    };
    println!("\nSubscriber A fetching Signed packet from Subscriber B...");
    {
        match subscriberA.receive_signed_packet(&previous_msg_link).await {
            Ok((_signer_pk, unwrapped_public, unwrapped_masked)) => {
                return Err(anyhow::anyhow!("\nSubscriber B message should have failed due to no permissions"))
            },
            Err(e) => {
                println!("  SubscriberA: Did not accept Subscriber B message correctly");
                print!("  SubscriberA: {}", subscriberA);
            }
            
        }
    }

    println!("\nSigned packet 2 - SubscriberA (subscriber A has Write permission");
    let previous_msg_link = {
        let (msg, seq) = subscriberA
            .send_signed_packet(&previous_msg_link, &public_payload, &masked_payload)
            .await?;
        println!("  msg => <{}> <{:x}>", msg.msgid, msg.to_msg_index());
        assert!(seq.is_none());
        print!("  SubscriberA: {}", subscriberA);
        msg
    };

    // TODO this fails due to double messages in state because of the Illegal Mr. Subscriber B
    println!("\nAuthor fetching transactions...");
    utils::fetch_next_messages(&mut author).await
}
