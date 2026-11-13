// Hand-maintained to mirror migrations/*/up.sql.
diesel::table! {
    roles (id) {
        id -> Uuid,
        name -> Text,
        description -> Text,
        permissions -> Jsonb,
        builtin -> Bool,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    users (id) {
        id -> Uuid,
        username -> Text,
        display_name -> Text,
        email -> Nullable<Text>,
        password_hash -> Nullable<Text>,
        role_id -> Uuid,
        auth_source -> Text,
        is_fallback_admin -> Bool,
        disabled -> Bool,
        must_change_password -> Bool,
        failed_logins -> Int4,
        locked_until -> Nullable<Timestamptz>,
        language -> Text,
        created_at -> Timestamptz,
        last_login_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    dashboard_layouts (user_id) {
        user_id -> Uuid,
        layout -> Jsonb,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    settings (key) {
        key -> Text,
        value -> Jsonb,
        encrypted -> Bool,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    host_groups (id) {
        id -> Uuid,
        name -> Text,
        description -> Text,
        created_at -> Timestamptz,
        color -> Text,
    }
}

diesel::table! {
    hosts (id) {
        id -> Uuid,
        hostname -> Text,
        display_name -> Nullable<Text>,
        status -> Text,
        public_key_pem -> Text,
        facts -> Jsonb,
        os_family -> Text,
        distro -> Text,
        package_manager -> Text,
        asset_type -> Text,
        environment -> Text,
        criticality -> Text,
        internet_exposed -> Bool,
        has_staging -> Bool,
        ring -> Int4,
        tags -> Jsonb,
        comm_mode -> Text,
        connected -> Bool,
        eco -> Bool,
        reboot_required -> Bool,
        downtime_until -> Nullable<Timestamptz>,
        heartbeat_secs -> Int4,
        last_seen_at -> Nullable<Timestamptz>,
        last_inventory_at -> Nullable<Timestamptz>,
        enrolled_at -> Timestamptz,
        created_at -> Timestamptz,
        modules -> Jsonb,
        metrics -> Jsonb,
        metrics_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    host_group_members (host_id, group_id) {
        host_id -> Uuid,
        group_id -> Uuid,
    }
}

diesel::table! {
    enrollment_tokens (id) {
        id -> Uuid,
        token_hash -> Text,
        label -> Text,
        group_id -> Nullable<Uuid>,
        environment -> Text,
        comm_mode -> Text,
        expires_at -> Timestamptz,
        max_uses -> Int4,
        used_count -> Int4,
        revoked -> Bool,
        created_by -> Text,
        created_at -> Timestamptz,
        modules -> Nullable<Jsonb>,
    }
}

diesel::table! {
    host_packages (host_id, name, source) {
        host_id -> Uuid,
        name -> Text,
        source -> Text,
        version -> Text,
        candidate -> Nullable<Text>,
        is_security -> Bool,
        cves -> Jsonb,
        meta -> Jsonb,
    }
}

diesel::table! {
    vulnerabilities (cve_id) {
        cve_id -> Text,
        cvss -> Nullable<Float8>,
        severity -> Text,
        summary -> Text,
        exploited -> Bool,
        published_at -> Nullable<Timestamptz>,
        source -> Text,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    policies (id) {
        id -> Uuid,
        name -> Text,
        description -> Text,
        enabled -> Bool,
        priority -> Int4,
        definition -> Jsonb,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    policy_groups (policy_id, group_id) {
        policy_id -> Uuid,
        group_id -> Uuid,
    }
}

diesel::table! {
    tasks (id) {
        id -> Uuid,
        host_id -> Uuid,
        kind -> Text,
        status -> Text,
        policy_id -> Nullable<Uuid>,
        parent_task_id -> Nullable<Uuid>,
        packages -> Jsonb,
        decision -> Nullable<Jsonb>,
        exec -> Nullable<Jsonb>,
        result -> Nullable<Jsonb>,
        message -> Text,
        requested_by -> Text,
        approved_by -> Nullable<Text>,
        scheduled_for -> Timestamptz,
        sent_at -> Nullable<Timestamptz>,
        started_at -> Nullable<Timestamptz>,
        finished_at -> Nullable<Timestamptz>,
        soak_until -> Nullable<Timestamptz>,
        batch_key -> Nullable<Text>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        deadline_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    task_events (id) {
        id -> Int8,
        task_id -> Uuid,
        ts -> Timestamptz,
        status -> Text,
        message -> Text,
    }
}

diesel::table! {
    waivers (id) {
        id -> Uuid,
        host_id -> Nullable<Uuid>,
        group_id -> Nullable<Uuid>,
        cve_id -> Nullable<Text>,
        package -> Nullable<Text>,
        reason -> Text,
        compensating_controls -> Text,
        requested_by -> Text,
        expires_at -> Timestamptz,
        status -> Text,
        ciso_approved_by -> Nullable<Text>,
        ciso_approved_at -> Nullable<Timestamptz>,
        business_approved_by -> Nullable<Text>,
        business_approved_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    audit_log (seq) {
        seq -> Int8,
        ts -> Timestamptz,
        actor -> Text,
        action -> Text,
        target_type -> Text,
        target_id -> Text,
        host_id -> Nullable<Uuid>,
        details -> Jsonb,
        ip -> Text,
        prev_hash -> Text,
        hash -> Text,
    }
}

diesel::joinable!(users -> roles (role_id));
diesel::joinable!(host_group_members -> hosts (host_id));
diesel::joinable!(host_group_members -> host_groups (group_id));
diesel::joinable!(host_packages -> hosts (host_id));
diesel::table! {
    notification_channels (id) {
        id -> Uuid,
        name -> Text,
        kind -> Text,
        config -> Jsonb,
        secret -> Text,
        enabled -> Bool,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    notification_rules (id) {
        id -> Uuid,
        name -> Text,
        enabled -> Bool,
        events -> Jsonb,
        min_severity -> Text,
        frequency -> Text,
        digest_hour -> Int4,
        group_ids -> Jsonb,
        channel_ids -> Jsonb,
        roles -> Jsonb,
        emails -> Jsonb,
        last_digest_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    notifications (id) {
        id -> Uuid,
        rule_id -> Nullable<Uuid>,
        channel_id -> Uuid,
        event -> Text,
        severity -> Text,
        title -> Text,
        body -> Text,
        payload -> Jsonb,
        recipients -> Jsonb,
        status -> Text,
        attempts -> Int4,
        last_error -> Text,
        next_attempt_at -> Timestamptz,
        created_at -> Timestamptz,
        sent_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    api_tokens (id) {
        id -> Uuid,
        user_id -> Uuid,
        name -> Text,
        token_hash -> Text,
        prefix -> Text,
        expires_at -> Nullable<Timestamptz>,
        last_used_at -> Nullable<Timestamptz>,
        revoked -> Bool,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    portal_apps (id) {
        id -> Uuid,
        name -> Text,
        version -> Text,
        category -> Text,
        description -> Text,
        icon -> Text,
        group_ids -> Jsonb,
        enabled -> Bool,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        recipes -> Jsonb,
    }
}

diesel::joinable!(api_tokens -> users (user_id));
diesel::joinable!(policy_groups -> policies (policy_id));
diesel::joinable!(policy_groups -> host_groups (group_id));
diesel::joinable!(tasks -> hosts (host_id));
diesel::joinable!(task_events -> tasks (task_id));

diesel::allow_tables_to_appear_in_same_query!(
    roles, users, dashboard_layouts, settings, host_groups, hosts, host_group_members,
    enrollment_tokens, host_packages, vulnerabilities, policies, policy_groups, tasks,
    task_events, waivers, audit_log, notification_channels, notification_rules, notifications, api_tokens, portal_apps,
);

diesel::table! {
    reports (id) {
        id -> Uuid,
        name -> Text,
        enabled -> Bool,
        frequency -> Text,
        weekday -> Int4,
        day_of_month -> Int4,
        send_at -> Text,
        timezone -> Text,
        language -> Text,
        sections -> Jsonb,
        group_ids -> Jsonb,
        emails -> Jsonb,
        channel_ids -> Jsonb,
        skip_if_empty -> Bool,
        last_run_at -> Nullable<Timestamptz>,
        last_slot_at -> Nullable<Timestamptz>,
        attempts -> Int4,
        last_status -> Text,
        last_error -> Text,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    report_runs (id) {
        id -> Uuid,
        report_id -> Uuid,
        at -> Timestamptz,
        trigger -> Text,
        status -> Text,
        error -> Text,
        recipients -> Jsonb,
        title -> Text,
    }
}

diesel::joinable!(report_runs -> reports (report_id));
diesel::allow_tables_to_appear_in_same_query!(reports, report_runs);

diesel::table! {
    host_metrics (host_id, at) {
        host_id -> Uuid,
        at -> Timestamptz,
        cpu_pct -> Nullable<Float4>,
        mem_pct -> Nullable<Float4>,
        load_1m -> Nullable<Float4>,
    }
}

diesel::joinable!(host_metrics -> hosts (host_id));
diesel::allow_tables_to_appear_in_same_query!(host_metrics, hosts);
