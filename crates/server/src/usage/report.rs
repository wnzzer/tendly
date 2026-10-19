//! The usage report: calls summed by day, model, client, conversation and
//! hour of the week, narrowed by an optional filter.

use super::*;

#[derive(Debug, Default, Serialize)]
pub struct Bucket {
    #[serde(flatten)]
    pub tokens: Tokens,
    pub total: u64,
    pub cost: f64,
    /// Tokens from models without a published price, left out of `cost`.
    pub unpriced: u64,
    pub calls: u64,
    /// Messages the person sent.
    pub user_messages: u64,
    /// Five-minute slots with anything happening, in minutes.
    pub active_minutes: u64,
    /// Requests answered, from clients that log the ones that fail, and
    /// requests that failed: a failure rate is `failed / (answered + failed)`.
    pub answered: u64,
    pub failed: u64,
    /// Attempts that failed and were tried again, on top of either.
    pub retried: u64,
    /// The output of the calls whose duration is known, and that duration:
    /// tokens a second, waiting for the first included.
    pub timed_output: u64,
    pub timed_ms: u64,
}
impl Bucket {
    fn add(&mut self, call: &Call, prices: &Prices) {
        if call.user {
            self.user_messages += 1;
            return;
        }
        match call.outcome {
            Outcome::Retried => {
                self.retried += 1;
                return;
            }
            Outcome::Failed => self.failed += 1,
            Outcome::Answered => self.answered += 1,
            Outcome::Unlogged => {}
        }
        self.tokens.add(&call.tokens);
        self.total += call.tokens.total();
        match prices.cost(call) {
            Some(value) => self.cost += value,
            None => self.unpriced += call.tokens.total(),
        }
        if call.outcome != Outcome::Failed {
            self.calls += 1;
        }
        if let Some(ms) = call.duration_ms {
            self.timed_output += call.tokens.output;
            self.timed_ms += ms;
        }
    }
}

#[derive(Serialize)]
pub struct DayRow {
    pub date: String,
    #[serde(flatten)]
    pub bucket: Bucket,
}
#[derive(Serialize)]
pub struct ModelRow {
    pub model: String,
    pub provider: ProviderKind,
    /// The price this model is costed at, and whether the user set it.
    pub price: Option<Price>,
    pub custom_price: bool,
    #[serde(flatten)]
    pub bucket: Bucket,
}
#[derive(Serialize)]
pub struct ConversationRow {
    pub conversation: String,
    pub provider: ProviderKind,
    /// The Tendly session this conversation belongs to, when known.
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub cwd: String,
    pub models: Vec<String>,
    pub first_at: i64,
    pub last_at: i64,
    #[serde(flatten)]
    pub bucket: Bucket,
}
#[derive(Serialize)]
pub struct Report {
    pub from: i64,
    pub to: i64,
    pub totals: Bucket,
    /// The same length of time just before `from`, for a trend.
    pub previous: Bucket,
    pub days: Vec<DayRow>,
    pub models: Vec<ModelRow>,
    pub providers: Vec<ModelRow>,
    /// Tokens by local weekday (0 = Monday) and hour: 7 x 24.
    pub heatmap: Vec<Vec<u64>>,
    pub heatmap_cost: Vec<Vec<f64>>,
    /// Active minutes by weekday and hour.
    pub heatmap_active: Vec<Vec<u64>>,
    /// Summed span of every conversation, first event to last, in seconds.
    pub duration_seconds: i64,
    pub previous_duration_seconds: i64,
    /// What there is to filter by in this range, before any filter.
    pub options: Options,
    pub conversations: Vec<ConversationRow>,
    pub conversation_count: usize,
    pub directories: Vec<(String, u64)>,
    /// When the published prices in use were last updated, and whether they
    /// came with this build or were fetched since.
    pub prices_updated_at: String,
    pub prices_origin: crate::price_list::Origin,
}

#[derive(Serialize, Default)]
pub struct Options {
    pub providers: Vec<ProviderKind>,
    pub models: Vec<String>,
    pub projects: Vec<String>,
}

/// Narrow the report to one client, one model or one project directory.
#[derive(Default, Clone)]
pub struct Filter {
    pub provider: Option<ProviderKind>,
    pub model: Option<String>,
    pub project: Option<String>,
}
impl Filter {
    fn keeps(&self, call: &Call) -> bool {
        self.provider.is_none_or(|provider| provider == call.provider)
            // A message belongs to a conversation, not a model: keep it under a model filter.
            && self.model.as_deref().is_none_or(|model| call.user || &*call.model == model)
            && self.project.as_deref().is_none_or(|project| {
                &*call.cwd == project || call.cwd.starts_with(&format!("{}/", project.trim_end_matches('/')))
            })
    }
}

/// Conversations met while reading, with the models each used.
type Conversations = HashMap<(ProviderKind, Arc<str>), (ConversationRow, HashSet<Arc<str>>)>;

pub struct SessionLink {
    pub id: String,
    pub title: String,
}

pub(super) fn report(
    calls: &[Call],
    from: i64,
    to: i64,
    tz_minutes: i32,
    links: &HashMap<String, SessionLink>,
    prices: &Prices,
    filter: &Filter,
) -> Report {
    let offset = FixedOffset::east_opt(tz_minutes.clamp(-14 * 60, 14 * 60) * 60)
        .unwrap_or(FixedOffset::east_opt(0).unwrap());
    let local = |at: i64| {
        DateTime::<Utc>::from_timestamp(at, 0)
            .unwrap_or_default()
            .with_timezone(&offset)
    };
    let span = (to - from).max(0);
    let mut totals = Bucket::default();
    let mut previous = Bucket::default();
    let mut days: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut models: HashMap<(String, ProviderKind), Bucket> = HashMap::new();
    let mut providers: HashMap<ProviderKind, Bucket> = HashMap::new();
    let mut heatmap = vec![vec![0u64; 24]; 7];
    let mut heatmap_cost = vec![vec![0f64; 24]; 7];
    // Activity is counted in five-minute slots, so a burst of calls is one slot.
    let slot = |at: i64| at.div_euclid(300);
    let mut active: HashSet<i64> = HashSet::new();
    let mut previous_active: HashSet<i64> = HashSet::new();
    let mut previous_spans: HashMap<(ProviderKind, Arc<str>), (i64, i64)> = HashMap::new();
    let mut options_providers: HashSet<ProviderKind> = HashSet::new();
    let mut options_models: HashMap<String, u64> = HashMap::new();
    let mut options_projects: HashMap<Arc<str>, u64> = HashMap::new();
    let mut conversations: Conversations = HashMap::new();
    let mut directories: HashMap<Arc<str>, u64> = HashMap::new();
    // Every day of the range has a row, so a quiet day shows as zero, not a gap.
    let mut day = local(from).date_naive();
    let last = local(to).date_naive();
    while day <= last && days.len() < 400 {
        days.insert(day.format("%Y-%m-%d").to_string(), Bucket::default());
        day += ChronoDuration::days(1);
    }
    for call in calls {
        let in_range = call.at >= from && call.at < to;
        if in_range {
            options_providers.insert(call.provider);
            if !call.user && !call.model.is_empty() {
                *options_models.entry(call.model.to_string()).or_default() += call.tokens.total();
            }
            if !call.cwd.is_empty() {
                *options_projects.entry(call.cwd.clone()).or_default() += call.tokens.total();
            }
        }
        if !filter.keeps(call) {
            continue;
        }
        if call.at >= from - span && call.at < from {
            previous.add(call, prices);
            previous_active.insert(slot(call.at));
            let span = previous_spans
                .entry((call.provider, call.conversation.clone()))
                .or_insert((call.at, call.at));
            span.0 = span.0.min(call.at);
            span.1 = span.1.max(call.at);
            continue;
        }
        if !in_range {
            continue;
        }
        totals.add(call, prices);
        active.insert(slot(call.at));
        let at = local(call.at);
        days.entry(at.format("%Y-%m-%d").to_string())
            .or_default()
            .add(call, prices);
        // A failure before any model was named belongs to no model's row.
        let nameless_failure =
            call.model.is_empty() && matches!(call.outcome, Outcome::Failed | Outcome::Retried);
        if !call.user && !nameless_failure {
            models
                .entry((call.model.to_string(), call.provider))
                .or_default()
                .add(call, prices);
            let (weekday, hour) = (
                at.weekday().num_days_from_monday() as usize,
                at.hour() as usize,
            );
            heatmap[weekday][hour] += call.tokens.total();
            heatmap_cost[weekday][hour] += prices.cost(call).unwrap_or(0.0);
            *directories.entry(call.cwd.clone()).or_default() += call.tokens.total();
        }
        providers
            .entry(call.provider)
            .or_default()
            .add(call, prices);
        let link = call
            .home_session
            .as_deref()
            .and_then(|id| {
                links
                    .get(id)
                    .map(|link| (id.to_string(), link.title.clone()))
            })
            .or_else(|| {
                links
                    .get(&*call.conversation)
                    .map(|link| (link.id.clone(), link.title.clone()))
            });
        let (row, model_set) = conversations
            .entry((call.provider, call.conversation.clone()))
            .or_insert_with(|| {
                (
                    ConversationRow {
                        conversation: call.conversation.to_string(),
                        provider: call.provider,
                        session_id: link.as_ref().map(|(id, _)| id.clone()),
                        title: link.as_ref().map(|(_, title)| title.clone()),
                        cwd: call.cwd.to_string(),
                        models: Vec::new(),
                        first_at: call.at,
                        last_at: call.at,
                        bucket: Bucket::default(),
                    },
                    HashSet::new(),
                )
            });
        row.first_at = row.first_at.min(call.at);
        row.last_at = row.last_at.max(call.at);
        row.bucket.add(call, prices);
        if !call.user && !call.model.is_empty() && model_set.insert(call.model.clone()) {
            row.models.push(call.model.to_string());
        }
    }
    totals.active_minutes = active.len() as u64 * 5;
    previous.active_minutes = previous_active.len() as u64 * 5;
    let mut heatmap_active = vec![vec![0u64; 24]; 7];
    for slot in &active {
        let at = local(slot * 300);
        heatmap_active[at.weekday().num_days_from_monday() as usize][at.hour() as usize] += 5;
        if let Some(day) = days.get_mut(&at.format("%Y-%m-%d").to_string()) {
            day.active_minutes += 5;
        }
    }
    let duration_seconds = conversations
        .values()
        .map(|(row, _)| row.last_at - row.first_at)
        .sum();
    let previous_duration_seconds = previous_spans
        .values()
        .map(|(first, last)| last - first)
        .sum();
    let mut options_models: Vec<(String, u64)> = options_models.into_iter().collect();
    options_models.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    let mut options_projects: Vec<(Arc<str>, u64)> = options_projects.into_iter().collect();
    options_projects.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    let options = Options {
        providers: ProviderKind::AGENTS
            .into_iter()
            .filter(|provider| options_providers.contains(provider))
            .collect(),
        models: options_models
            .into_iter()
            .map(|(model, _)| model)
            .take(40)
            .collect(),
        projects: options_projects
            .into_iter()
            .map(|(cwd, _)| cwd.to_string())
            .take(40)
            .collect(),
    };
    let mut models: Vec<ModelRow> = models
        .into_iter()
        .map(|((model, provider), bucket)| {
            let priced = prices.of(&model);
            ModelRow {
                price: priced.map(|(price, _)| price),
                custom_price: priced.is_some_and(|(_, custom)| custom),
                model,
                provider,
                bucket,
            }
        })
        .collect();
    models.sort_by_key(|row| std::cmp::Reverse(row.bucket.total));
    let mut providers: Vec<ModelRow> = providers
        .into_iter()
        .map(|(provider, bucket)| ModelRow {
            model: String::new(),
            provider,
            price: None,
            custom_price: false,
            bucket,
        })
        .collect();
    providers.sort_by_key(|row| std::cmp::Reverse(row.bucket.total));
    let conversation_count = conversations.len();
    let mut conversations: Vec<ConversationRow> =
        conversations.into_values().map(|(row, _)| row).collect();
    conversations.sort_by_key(|row| std::cmp::Reverse(row.bucket.total));
    conversations.truncate(100);
    let mut directories: Vec<(String, u64)> = directories
        .into_iter()
        .map(|(cwd, total)| (cwd.to_string(), total))
        .collect();
    directories.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    directories.truncate(20);
    let (prices_updated_at, prices_origin) = crate::price_list::status();
    Report {
        from,
        to,
        totals,
        previous,
        days: days
            .into_iter()
            .map(|(date, bucket)| DayRow { date, bucket })
            .collect(),
        models,
        providers,
        heatmap,
        heatmap_cost,
        heatmap_active,
        duration_seconds,
        previous_duration_seconds,
        options,
        conversations,
        conversation_count,
        directories,
        prices_updated_at,
        prices_origin,
    }
}
