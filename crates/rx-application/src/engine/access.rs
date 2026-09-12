use super::*;

pub(super) fn authorize(
    tx: &mut dyn Transaction,
    identity: &Identity,
    installation: &Installation,
    now: &TimePoint,
    cell: Option<&Name>,
    role: Role,
    terminal_required: bool,
) -> Result<Principal> {
    let (_, session): (_, Session) = load(tx, "session", &identity.session, SESSION)
        .map_err(|e| missing_as(e, Reject::Unauthenticated))?;
    if !session.active
        || session.principal != identity.principal
        || session.runtime_boot != installation.runtime_boot
        || session.expires_at.clock_id != now.clock_id
        || session.expires_at.ticks_ns <= now.ticks_ns
    {
        return reject(Reject::Unauthenticated);
    }
    let (_, mut principal): (_, Principal) = load(tx, "principal", &identity.principal, PRINCIPAL)?;
    if !principal.active
        || !principal.roles.contains(&role)
        || cell.is_some_and(|c| !principal.cells.contains(c))
    {
        return reject(Reject::Forbidden);
    }
    match (&session.terminal, &identity.terminal) {
        (Some(binding), Some((terminal_id, certificate))) => {
            if binding.id != *terminal_id || binding.certificate_digest != *certificate {
                return reject(Reject::Forbidden);
            }
            let (revision, terminal): (_, Terminal) =
                load(tx, "terminal", terminal_id, TERMINAL)
                    .map_err(|e| missing_as(e, Reject::Forbidden))?;
            if binding.revision != revision
                || !terminal.active
                || terminal.certificate_digest != *certificate
            {
                return reject(Reject::Forbidden);
            }
            principal.cells.retain(|c| terminal.cells.contains(c));
            if cell.is_some_and(|c| !principal.cells.contains(c)) {
                return reject(Reject::Forbidden);
            }
        }
        (None, None) if !terminal_required => {}
        _ => return reject(Reject::Forbidden),
    }
    Ok(principal)
}

pub(super) fn authorize_read(
    tx: &mut dyn Transaction,
    identity: &Identity,
    installation: &Installation,
    now: &TimePoint,
    cell: &Name,
) -> Result<Principal> {
    let p = authorize_identity(tx, identity, installation, now)?;
    if !p.cells.contains(cell) {
        return reject(Reject::Forbidden);
    }
    Ok(p)
}

pub(super) fn authorize_identity(
    tx: &mut dyn Transaction,
    identity: &Identity,
    installation: &Installation,
    now: &TimePoint,
) -> Result<Principal> {
    let (_, p): (_, Principal) = load(tx, "principal", &identity.principal, PRINCIPAL)
        .map_err(|e| missing_as(e, Reject::Unauthenticated))?;
    let role = p
        .roles
        .iter()
        .next()
        .copied()
        .ok_or(StoreError::Rejected(Reject::Forbidden))?;
    authorize(tx, identity, installation, now, None, role, false)
}

pub(super) fn current_session(
    tx: &mut dyn Transaction,
    principal: &Name,
    installation: &Installation,
    now: &TimePoint,
    role: Role,
    cell: &Name,
) -> Result<Session> {
    let mut current = None;
    for record in tx.scan("session/")? {
        let s: Session = decode(&record, SESSION)?;
        if s.principal == *principal
            && s.active
            && s.runtime_boot == installation.runtime_boot
            && s.expires_at.clock_id == now.clock_id
            && s.expires_at.ticks_ns > now.ticks_ns
        {
            if current.is_some() {
                return reject(Reject::Unauthenticated);
            }
            authorize(
                tx,
                &Identity {
                    principal: principal.clone(),
                    session: s.id.clone(),
                    terminal: None,
                },
                installation,
                now,
                Some(cell),
                role,
                false,
            )?;
            current = Some(s);
        }
    }
    current.ok_or(StoreError::Rejected(Reject::Unauthenticated))
}
