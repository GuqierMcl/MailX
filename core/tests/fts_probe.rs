//! F0 probe for local full-text search (docs/architecture/storage-search.md §5.1).
//!
//! Not part of normal runs. Rerun with:
//! `cargo test -p mailx-core --release --test fts_probe -- --ignored --nocapture --test-threads=1`
//! Set `FTS_PROBE_SIZES=10000,50000` to change the corpus sizes.
use rusqlite::{Connection, params};
use std::{
    fmt::Write as _,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const TOKENIZE: &str = "trigram case_sensitive 0 remove_diacritics 1";
const BODY_CAP: usize = 64 * 1024;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
    fn pick<'a, T: ?Sized>(&mut self, items: &[&'a T]) -> &'a T {
        items[self.below(items.len())]
    }
}

const ZH_SHORT: &[&str] = &[
    "发票", "会议", "合同", "报销", "项目", "通知", "客户", "订单", "报价", "付款", "审批", "日程",
    "方案", "周报", "预算", "文件", "附件", "请假", "面试", "测试",
];
const ZH_LONG: &[&str] = &[
    "财务报表",
    "采购订单",
    "项目进度",
    "季度总结",
    "供应商",
    "技术方案",
    "会议纪要",
    "合同审批",
    "报销流程",
    "客户反馈",
];
const ZH_CHARS: &str = "的一是在不了有和人这中大为上个国我以要他时来用们生到作地于出就分对成会可主发年动同工也能下过子说产种面而方后多定行学法所民得经十三之进着等部度家电力里如水化高自二理起小物现实加量都两体制机当使点从业本去把性好应开它合还因由其些然前外天政四日那社义事平形相全表间样与关各重新线内数正心反你明看原又么利比或但质气第向道命此变条只没结解问意建月公无系军很情者最立代想已通并提直题党程展五果料象员革位入常文总次品式活设及管特件长求老头基资边流路级少图山统接知较将组见计别她手角期根论运农指几九区强放决西被干做必战先回则任取据处队南给色光门即保治北造百规热领七海口东导器压志世金增争济阶油思术极交受联什认六共权收证改清己美再采转更单风切打白教速花带安场身车例真务具万每目至达走积示议声报斗完类八离华名确才科张信马节话米整空元况今集温传土许步群广石记需段研界拉林律叫且究观越织装影算低持音众书布复容儿须际商非验连断深难近矿千周委素技备半办青省列习响约支般史感劳便团往酸历市克何除消构府称太准精值号率族维划选标写存候毛亲快效斯院查江型眼王按格养易置派层片始却专状育厂京识适属圆包火住调满县局照参红细引听该铁价严首底液官德随病苏失尔死讲配女黄推显谈罪神艺呢席含企望密批营项防举球英氧势告李台落木帮轮破亚师围注远字材排供河态封另施减树溶怎止案言士均武固叶鱼波视仅费紧爱左章早朝害续轻服试食充兵源判护司足某练差致板田降黑犯负击范继兴似余坚曲输修故城夫够送笔船占右财吃富春职觉汉画功巴跟虽杂飞检吸助升阳互初创抗考投坏策古径换未跑留钢曾端责站简述钱副尽帝射草冲承独令限阿宣环双请超微让控州良轴找否纪益依优顶础载倒房突坐粉敌略客袁冷胜绝析块剂测丝协诉念陈仍罗盐友洋错苦夜刑移频逐靠混母短皮终聚汽村云哪既距卫停烈央察烧迅境若印洲刻括激孔搞甚室待核校散侵吧甲游久菜味旧模湖货损预阻毫普稳乙妈植息扩银语挥酒守拿序纸医缺雨吗针刘啊急唱误训愿审附获茶鲜粮斤孩脱硫肥善龙演父渐血欢械掌歌沙刚攻谓盾讨晚粒乱燃矛乎杀药宁鲁贵钟煤读班伯香介迫句丰培握兰担弦蛋沉假穿执答乐谁顺烟缩征脸喜松脚困异免背星福买染井概慢怕磁倍祖皇促静补评翻肉践尼衣宽扬棉希伤操垂秋宜氢套督振架亮末宪庆编牛触映雷销诗座居抓裂胞呼娘景威绿晶厚盟衡鸡孙延危胶屋乡临陆顾掉呀灯岁措束耐剧玉赵跳哥季课凯胡额款绍卷齐伟蒸殖永宗苗川炉岩弱零杨奏沿露杆探滑镇饭浓航怀赶库夺伊灵税途灭赛归召鼓播盘裁险康唯录菌纯借糖盖横符私努堂域枪润幅哈竟熟虫泽脑壤碳欧遍侧寨敢彻虑斜薄庭纳弹饲伸折麦湿暗荷瓦塞床筑恶户访塔奇透梁刀旋迹卡氯遇份毒泥退洗摆灰彩卖耗夏择忙铜献硬予繁圈雪函亦抽篇阵阴丁尺追堆雄迎泛爸楼避谋吨野猪旗累偏典馆索秦脂潮爷豆忽托惊塑遗愈朱替纤粗倾尚痛楚谢奋购磨君池旁碎骨监捕弟暴割贯殊释词亡壁顿宝午尘闻揭炮残冬桥妇警综招吴付浮遭徐您摇谷赞箱隔订男吹园纷唐败宋玻巨耕坦荣闭湾键凡驻锅救恩剥凝碱齿截炼麻纺禁废盛版缓净睛昌婚涉筒嘴插岸朗庄街藏姑贸腐奴啦惯乘伙恢匀纱扎辩耳彪臣亿璃抵脉秀萨俄网舞店喷纵寸汗挂洪贺闪柬爆烯津稻墙软勇像滚厘蒙芳肯坡柱荡腾";
const EN_WORDS: &[&str] = &[
    "invoice",
    "meeting",
    "contract",
    "reimbursement",
    "project",
    "notice",
    "customer",
    "order",
    "quote",
    "payment",
    "approval",
    "schedule",
    "proposal",
    "weekly",
    "budget",
    "document",
    "attachment",
    "leave",
    "interview",
    "testing",
    "quarterly",
    "report",
    "update",
    "review",
    "please",
    "thanks",
    "regarding",
    "follow",
    "action",
    "items",
    "status",
    "release",
    "deploy",
    "server",
    "database",
    "migration",
    "security",
    "account",
    "password",
    "reset",
    "welcome",
    "newsletter",
    "unsubscribe",
    "shipping",
    "delivery",
    "tracking",
    "receipt",
    "subscription",
    "renewal",
    "discount",
    "offer",
    "team",
    "design",
    "feedback",
    "roadmap",
    "sprint",
    "ticket",
    "issue",
    "comment",
    "merge",
    "request",
    "build",
    "failed",
    "passed",
    "alert",
    "monitor",
    "travel",
    "booking",
    "confirmation",
    "flight",
    "hotel",
    "agenda",
    "minutes",
    "draft",
    "final",
    "version",
    "policy",
    "legal",
    "terms",
    "privacy",
    "survey",
    "results",
    "analysis",
    "forecast",
    "revenue",
    "expense",
    "audit",
    "compliance",
    "onboarding",
    "training",
    "cafe",
];
const SYLLABLES: &[&str] = &[
    "ka", "to", "mi", "ren", "sol", "vex", "dar", "lin", "pol", "qua", "bet", "zan", "ner", "fil",
    "gor", "hap", "jun", "kel", "mor", "nit",
];
const EN_FIRST: &[&str] = &[
    "John", "Mary", "Alice", "Bob", "Carol", "David", "Emma", "Frank",
];
const EN_LAST: &[&str] = &[
    "Smith", "Johnson", "Brown", "Taylor", "Wilson", "Moore", "Clark",
];
const ZH_FIRST: &[&str] = &["张", "王", "李", "赵", "刘", "陈", "杨", "黄"];
const ZH_GIVEN: &[&str] = &["伟", "芳", "娜", "敏", "静", "磊", "洋", "艳", "勇", "杰"];
const PINYIN: &[&str] = &[
    "zhang", "wang", "li", "zhao", "liu", "chen", "yang", "huang",
];
const DOMAINS: &[&str] = &[
    "example.com",
    "acme.io",
    "mail.test",
    "corp.example.cn",
    "qq.com",
];
const EXT: &[&str] = &["pdf", "xlsx", "docx", "png", "zip"];

struct Corpus {
    zh_chars: Vec<char>,
}
impl Corpus {
    fn new() -> Self {
        Self {
            zh_chars: ZH_CHARS.chars().collect(),
        }
    }
    fn zh_word(&self, rng: &mut Rng) -> String {
        if rng.chance(6) {
            return rng.pick(ZH_SHORT).to_string();
        }
        if rng.chance(3) {
            return rng.pick(ZH_LONG).to_string();
        }
        let n = 2 + rng.below(2);
        (0..n)
            .map(|_| self.zh_chars[rng.below(self.zh_chars.len())])
            .collect()
    }
    fn en_word(rng: &mut Rng) -> String {
        if rng.chance(30) {
            let n = 2 + rng.below(3);
            (0..n).map(|_| rng.pick(SYLLABLES)).collect()
        } else {
            rng.pick(EN_WORDS).to_string()
        }
    }
    /// Text of at least `min_bytes` bytes (UTF-8); `lang` 0 = zh, 1 = en, 2 = mixed.
    fn text_bytes(&self, rng: &mut Rng, lang: u8, min_bytes: usize) -> String {
        let mut out = String::new();
        let mut zh_run = lang != 1;
        while out.len() < min_bytes {
            if lang == 2 && rng.chance(15) {
                zh_run = !zh_run;
            }
            if zh_run {
                out.push_str(&self.zh_word(rng));
                if rng.chance(12) {
                    out.push_str(if rng.chance(50) { "，" } else { "。" });
                }
            } else {
                out.push_str(&Self::en_word(rng));
                out.push(if rng.chance(10) { '.' } else { ' ' });
                if out.ends_with('.') {
                    out.push(' ');
                }
            }
            if lang == 2 && zh_run && rng.chance(20) {
                out.push(' ');
            }
        }
        out
    }
    /// Text of `chars` characters.
    fn text_chars(&self, rng: &mut Rng, lang: u8, chars: usize) -> String {
        let mut t = self.text_bytes(rng, lang, chars * 3);
        let cut = t
            .char_indices()
            .nth(chars)
            .map_or(t.len(), |(index, _)| index);
        t.truncate(cut);
        t.trim().to_string()
    }
}

struct Row {
    subject: String,
    people: String,
    preview: String,
    body: String,
    attachments: String,
    received_at: i64,
}
impl Row {
    fn raw_bytes(&self) -> usize {
        self.subject.len()
            + self.people.len()
            + self.preview.len()
            + self.body.len()
            + self.attachments.len()
    }
}

fn gen_row(corpus: &Corpus, rng: &mut Rng, index: usize, body_percent: usize) -> Row {
    let lang = match rng.below(100) {
        0..=54 => 0,
        55..=89 => 1,
        _ => 2,
    };
    let subject_len = 10 + rng.below(31);
    let subject = corpus.text_chars(rng, lang, subject_len);
    let people = (0..1 + rng.below(4))
        .map(|_| {
            let domain = rng.pick(DOMAINS);
            if rng.chance(50) {
                let first = rng.below(ZH_FIRST.len());
                format!(
                    "{}{} <{}{}@{domain}>",
                    ZH_FIRST[first],
                    rng.pick(ZH_GIVEN),
                    PINYIN[first],
                    rng.below(100)
                )
            } else {
                let (f, l) = (rng.pick(EN_FIRST), rng.pick(EN_LAST));
                format!(
                    "{f} {l} <{}.{}@{domain}>",
                    f.to_lowercase(),
                    l.to_lowercase()
                )
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let preview = corpus.text_chars(rng, lang, 200);
    let mut body = String::new();
    if rng.chance(body_percent) {
        let body_len = 2048 + rng.below(18 * 1024 + 1);
        body = corpus.text_bytes(rng, lang, body_len);
        if body.len() > BODY_CAP {
            let mut cut = BODY_CAP;
            while !body.is_char_boundary(cut) {
                cut -= 1;
            }
            body.truncate(cut);
        }
    }
    let attachments = (0..rng.below(4))
        .map(|_| {
            let stem = if rng.chance(50) {
                format!("{}_{}", rng.pick(ZH_SHORT), 2020 + rng.below(6))
            } else {
                format!("{}_q{}", rng.pick(EN_WORDS), 1 + rng.below(4))
            };
            format!("{stem}.{}", rng.pick(EXT))
        })
        .collect::<Vec<_>>()
        .join("\n");
    Row {
        subject,
        people,
        preview,
        body,
        attachments,
        received_at: 1_700_000_000_000 + (index as i64) * 61_000 + rng.below(60_000) as i64,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Variant {
    None,
    Full,
    Contentless,
}
impl Variant {
    fn name(self) -> &'static str {
        match self {
            Self::None => "no-fts",
            Self::Full => "full",
            Self::Contentless => "contentless",
        }
    }
}

fn open(dir: &TempDir, name: &str, variant: Variant) -> Connection {
    let db = Connection::open(dir.path().join(name)).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.pragma_update(None, "synchronous", "FULL").unwrap();
    db.execute_batch(
        "CREATE TABLE messages(id TEXT PRIMARY KEY, received_at INTEGER NOT NULL, flags INTEGER NOT NULL DEFAULT 0);
         CREATE INDEX messages_received ON messages(received_at DESC, id DESC);
         CREATE TABLE fts_map(fts_rowid INTEGER PRIMARY KEY, email_id TEXT NOT NULL UNIQUE REFERENCES messages(id) ON DELETE CASCADE);",
    )
    .unwrap();
    let extra = match variant {
        Variant::None => return db,
        Variant::Full => "",
        Variant::Contentless => ", content=''",
    };
    db.execute_batch(&format!(
        "CREATE VIRTUAL TABLE fts USING fts5(subject, people, preview, body, attachments, tokenize='{TOKENIZE}'{extra});"
    ))
    .unwrap();
    db
}

struct Loaded {
    insert: Duration,
    raw_bytes: usize,
    db_bytes: u64,
}

fn load(
    db: &Connection,
    dir: &TempDir,
    name: &str,
    n: usize,
    body_percent: usize,
    v: Variant,
) -> Loaded {
    let corpus = Corpus::new();
    let mut rng = Rng(42);
    let mut raw = 0;
    let start = Instant::now();
    let mut done = 0;
    while done < n {
        let tx = db.unchecked_transaction().unwrap();
        {
            let mut msg = tx
                .prepare("INSERT INTO messages(id, received_at) VALUES (?1, ?2)")
                .unwrap();
            let mut map = tx
                .prepare("INSERT INTO fts_map(fts_rowid, email_id) VALUES (?1, ?2)")
                .unwrap();
            let mut fts = (v != Variant::None).then(|| {
                tx.prepare("INSERT INTO fts(rowid, subject, people, preview, body, attachments) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
                    .unwrap()
            });
            for i in done..(done + 5000).min(n) {
                let row = gen_row(&corpus, &mut rng, i, body_percent);
                raw += row.raw_bytes();
                let id = format!("m{i}");
                msg.execute(params![id, row.received_at]).unwrap();
                if let Some(fts) = fts.as_mut() {
                    let rowid = i as i64 + 1;
                    map.execute(params![rowid, id]).unwrap();
                    fts.execute(params![
                        rowid,
                        row.subject,
                        row.people,
                        row.preview,
                        row.body,
                        row.attachments
                    ])
                    .unwrap();
                }
            }
        }
        tx.commit().unwrap();
        done = (done + 5000).min(n);
    }
    let insert = start.elapsed();
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    let db_bytes = std::fs::metadata(dir.path().join(name)).unwrap().len();
    Loaded {
        insert,
        raw_bytes: raw,
        db_bytes,
    }
}

fn quote(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

fn like_pattern(term: &str) -> String {
    let mut out = String::from("%");
    for c in term.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

const LIKE_ALL: &str = "(subject LIKE ?1 ESCAPE '\\' OR people LIKE ?1 ESCAPE '\\' OR preview LIKE ?1 ESCAPE '\\' OR body LIKE ?1 ESCAPE '\\' OR attachments LIKE ?1 ESCAPE '\\')";
const LIKE_NO_BODY: &str = "(subject LIKE ?1 ESCAPE '\\' OR people LIKE ?1 ESCAPE '\\' OR preview LIKE ?1 ESCAPE '\\' OR attachments LIKE ?1 ESCAPE '\\')";

fn timed<F: FnMut() -> usize>(mut f: F) -> (f64, f64, usize) {
    let hits = f();
    let mut samples: Vec<f64> = (0..30)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    (samples[15], samples[28], hits)
}

fn run_query(db: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) -> usize {
    let mut stmt = db.prepare_cached(sql).unwrap();
    let mut rows = stmt.query(args).unwrap();
    let mut n = 0;
    while let Some(row) = rows.next().unwrap() {
        let _: String = row.get(0).unwrap();
        n += 1;
    }
    n
}

fn count(db: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) -> i64 {
    db.query_row(sql, args, |r| r.get(0)).unwrap()
}

fn join(where_clause: &str) -> String {
    format!(
        "SELECT m.id FROM fts JOIN fts_map x ON x.fts_rowid = fts.rowid JOIN messages m ON m.id = x.email_id WHERE {where_clause} ORDER BY m.received_at DESC, m.id DESC LIMIT 100"
    )
}
fn count_sql(where_clause: &str) -> String {
    format!("SELECT count(*) FROM fts WHERE {where_clause}")
}

fn bench_queries(db: &Connection, label: &str, full: bool) {
    let zh3 = quote("项目进度");
    let en3 = quote("quarterly");
    let and2 = format!("{} AND {}", quote("采购订单"), quote("invoice"));
    let phrase = quote("quarterly report");
    let zh2 = like_pattern("发票");
    let mixed_like = like_pattern("会议");
    println!("  -- {label}: query | median ms | p95 ms | matches (pre-LIMIT) | returned");
    let report = |name: &str, sql: String, csql: String, args: Vec<&dyn rusqlite::ToSql>| {
        let total = count(db, &csql, &args);
        let (med, p95, got) = timed(|| run_query(db, &sql, &args));
        println!("  {name:<44} {med:>8.2} {p95:>8.2} {total:>8} {got:>5}");
    };
    report(
        "MATCH zh>=3 (项目进度)",
        join("fts MATCH ?1"),
        count_sql("fts MATCH ?1"),
        vec![&zh3],
    );
    report(
        "MATCH en>=3 (quarterly)",
        join("fts MATCH ?1"),
        count_sql("fts MATCH ?1"),
        vec![&en3],
    );
    report(
        "MATCH AND 2 terms (采购订单 AND invoice)",
        join("fts MATCH ?1"),
        count_sql("fts MATCH ?1"),
        vec![&and2],
    );
    if !full {
        // contentless cannot run LIKE or snippet(). (detail=column was dropped from the probe:
        // a quoted multi-trigram term is a phrase query, which fails with "phrase queries are not supported".)
        return;
    }
    report(
        "MATCH phrase (\"quarterly report\")",
        join("fts MATCH ?1"),
        count_sql("fts MATCH ?1"),
        vec![&phrase],
    );
    let rare = like_pattern("槛楼");
    report(
        "LIKE 2-char RARE all cols (槛楼)",
        join(LIKE_ALL),
        count_sql(LIKE_ALL),
        vec![&rare],
    );
    report(
        "LIKE 2-char RARE excl. body (槛楼)",
        join(LIKE_NO_BODY),
        count_sql(LIKE_NO_BODY),
        vec![&rare],
    );
    let none = like_pattern("qx");
    report(
        "LIKE 2-char zero hits all cols (qx)",
        join(LIKE_ALL),
        count_sql(LIKE_ALL),
        vec![&none],
    );
    let mixed_rare_w = format!("fts MATCH ?1 AND {}", LIKE_ALL.replace("?1", "?2"));
    report(
        "MATCH(项目进度) AND LIKE RARE(槛楼) all cols",
        join(&mixed_rare_w),
        count_sql(&mixed_rare_w),
        vec![&zh3, &rare],
    );
    let rare_match = quote("采购订单");
    report(
        "MATCH rare-ish(采购订单) AND LIKE(会议) all",
        join(&mixed_rare_w),
        count_sql(&mixed_rare_w),
        vec![&rare_match, &mixed_like],
    );
    report(
        "LIKE 2-char all cols (发票)",
        join(LIKE_ALL),
        count_sql(LIKE_ALL),
        vec![&zh2],
    );
    report(
        "LIKE 2-char excl. body (发票)",
        join(LIKE_NO_BODY),
        count_sql(LIKE_NO_BODY),
        vec![&zh2],
    );
    let mixed_w = format!("fts MATCH ?1 AND {}", LIKE_ALL.replace("?1", "?2"));
    report(
        "MATCH(项目进度) AND LIKE(会议) all cols",
        join(&mixed_w),
        count_sql(&mixed_w),
        vec![&zh3, &mixed_like],
    );
    let mixed_nb = format!("fts MATCH ?1 AND {}", LIKE_NO_BODY.replace("?1", "?2"));
    report(
        "MATCH(项目进度) AND LIKE(会议) excl. body",
        join(&mixed_nb),
        count_sql(&mixed_nb),
        vec![&zh3, &mixed_like],
    );
}

fn bench_snippet(db: &Connection) {
    for (name, term) in [("zh 项目进度", "项目进度"), ("en quarterly", "quarterly")] {
        let q = quote(term);
        let plain = join("fts MATCH ?1");
        let ids_sql = "SELECT x.fts_rowid FROM fts JOIN fts_map x ON x.fts_rowid = fts.rowid JOIN messages m ON m.id = x.email_id WHERE fts MATCH ?1 ORDER BY m.received_at DESC, m.id DESC LIMIT 100";
        // Naive: snippet() in the sorted query is evaluated for every match, not only the top 100.
        let naive = "SELECT m.id, snippet(fts, -1, '[', ']', '…', 16) FROM fts JOIN fts_map x ON x.fts_rowid = fts.rowid JOIN messages m ON m.id = x.email_id WHERE fts MATCH ?1 ORDER BY m.received_at DESC, m.id DESC LIMIT 100";
        let (m0, p0, _) = timed(|| run_query(db, &plain, &[&q]));
        let (m1, p1, _) = timed(|| run_query(db, naive, &[&q]));
        // Two-step: page first, then snippet() only for those 100 rowids.
        let (m2, p2, _) = timed(|| {
            let ids: Vec<i64> = db
                .prepare_cached(ids_sql)
                .unwrap()
                .query_map([&q], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT rowid, snippet(fts, -1, '[', ']', '…', 16) FROM fts WHERE fts MATCH ?1 AND rowid IN ({list})"
            );
            let mut stmt = db.prepare(&sql).unwrap();
            let mut rows = stmt.query([&q]).unwrap();
            let mut n = 0;
            while let Some(row) = rows.next().unwrap() {
                let _: String = row.get(1).unwrap();
                n += 1;
            }
            n
        });
        println!(
            "  snippet() x100 [{name}] median/p95 ms: no snippet {m0:.2}/{p0:.2}; naive in-sorted-query {m1:.2}/{p1:.2}; two-step (page then snippet by rowid) {m2:.2}/{p2:.2}"
        );
    }
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}

fn sizes() -> Vec<usize> {
    std::env::var("FTS_PROBE_SIZES")
        .ok()
        .map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![10_000, 50_000])
}

#[test]
#[ignore = "F0 probe; run explicitly with --ignored --nocapture"]
fn fts_probe_runtime_and_correctness() {
    let db = Connection::open_in_memory().unwrap();
    let version: String = db
        .query_row("select sqlite_version()", [], |r| r.get(0))
        .unwrap();
    println!("sqlite_version = {version}");
    let opts: Vec<String> = db
        .prepare("PRAGMA compile_options")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .filter(|o: &String| o.contains("FTS"))
        .collect();
    println!("FTS compile options = {opts:?}");
    for tok in [
        "trigram case_sensitive 0 remove_diacritics 1",
        "trigram case_sensitive 0",
        "trigram",
    ] {
        let r = db.execute_batch(&format!(
            "CREATE VIRTUAL TABLE probe_{} USING fts5(subject, people, preview, body, attachments, tokenize='{tok}'); ",
            tok.len()
        ));
        println!(
            "tokenize='{tok}': {}",
            if r.is_ok() {
                "OK".into()
            } else {
                format!("{r:?}")
            }
        );
    }

    db.execute_batch(&format!(
        "CREATE VIRTUAL TABLE t USING fts5(subject, people, preview, body, attachments, tokenize='{TOKENIZE}');"
    ))
    .unwrap();
    let rows: &[(&str, &str, &str, &str)] = &[
        ("Invoice Q3 summary", "", "", ""),
        ("ＡＢＣ１２３ Report", "", "", ""),
        ("fullwidth haystack abc123 plain", "", "", ""),
        ("Café au lait menu", "", "", ""),
        ("naive cafe resume", "", "", ""),
        ("Contact", "Zhang San <zhang.san@example.com>", "", ""),
        ("say \"hello\" world", "", "", ""),
        ("a*b star", "", "", ""),
        ("foo AND bar logic", "", "", ""),
        ("NEAR(x y) proximity", "", "", ""),
        ("well-known minus", "", "", ""),
        ("key:value colon", "", "", ""),
        ("OR NOT keywords here", "", "", ""),
        ("项目进度汇报 发票", "", "", ""),
        ("100% sure a_b c\\d", "", "", ""),
        ("hidden", "", "", "body mentions cafeteria menu"),
    ];
    for (i, (s, p, pr, b)) in rows.iter().enumerate() {
        db.execute(
            "INSERT INTO t(rowid, subject, people, preview, body, attachments) VALUES (?1, ?2, ?3, ?4, ?5, '')",
            params![i as i64 + 1, s, p, pr, b],
        )
        .unwrap();
    }
    let m = |q: &str| -> Result<Vec<i64>, String> {
        let mut stmt = db
            .prepare("SELECT rowid FROM t WHERE t MATCH ?1 ORDER BY rowid")
            .map_err(|e| e.to_string())?;
        stmt.query_map([q], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .map(|r| r.map_err(|e| e.to_string()))
            .collect()
    };
    let check = |name: &str, ok: bool, detail: String| {
        println!("[{}] {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    };
    let r = m(&quote("INVOICE")).unwrap();
    check("case-insensitive en", r == [1], format!("{r:?}"));
    let r = m(&quote("项目进度")).unwrap();
    check("zh 4-char substring", r == [14], format!("{r:?}"));
    let r = m(&quote("目进")).unwrap();
    check(
        "MATCH 2-char returns nothing (why fallback is needed)",
        r.is_empty(),
        format!("{r:?}"),
    );
    let r = m(&quote("ab")).unwrap();
    check(
        "MATCH 2-char latin returns nothing",
        r.is_empty(),
        format!("{r:?}"),
    );
    println!(
        "[INFO] width folding: query 'abc123' -> {:?} (rows 2=fullwidth,3=halfwidth); query 'ＡＢＣ１２３' -> {:?}",
        m(&quote("abc123")).unwrap(),
        m(&quote("ＡＢＣ１２３")).unwrap()
    );
    println!(
        "[INFO] diacritics: 'cafe' -> {:?} (4=Café,5=cafe,16=cafeteria body); 'café' -> {:?}; 'CAFÉ' -> {:?}",
        m(&quote("cafe")).unwrap(),
        m(&quote("café")).unwrap(),
        m(&quote("CAFÉ")).unwrap()
    );
    let r = m(&quote("zhang.san@example.com")).unwrap();
    check("full email address", r == [6], format!("{r:?}"));
    let r = m(&quote("@example.com")).unwrap();
    check("domain with @", r == [6], format!("{r:?}"));
    let r = m(&quote("san@exa")).unwrap();
    check("partial across @", r == [6], format!("{r:?}"));
    let syntax: &[(&str, i64)] = &[
        ("say \"hello\" world", 7),
        ("\"", -1),
        ("a*b", 8),
        ("foo AND bar", 9),
        ("NEAR(x y)", 10),
        ("well-known", 11),
        ("key:value", 12),
        ("OR NOT", 13),
        ("subject:invoice", -1),
        ("-invoice", -1),
        ("invoice*", -1),
        ("(((", -1),
        ("^", -1),
        ("", -1),
    ];
    for (input, want) in syntax {
        let q = quote(input);
        match m(&q) {
            Ok(r) if *want > 0 => check(
                &format!("syntax literal {input:?} -> {q}"),
                r == [*want],
                format!("{r:?}"),
            ),
            Ok(r) => check(
                &format!("syntax literal {input:?} -> {q} (no error)"),
                r.is_empty(),
                format!("{r:?}"),
            ),
            Err(e) => check(
                &format!("syntax literal {input:?} -> {q}"),
                false,
                format!("ERROR {e}"),
            ),
        }
    }
    // Wrapped parts joined with AND, as the planned query builder would do.
    let q = format!("{} AND {}", quote("foo AND bar"), quote("logic"));
    let r = m(&q).unwrap();
    check(
        "wrapped terms joined with AND",
        r == [9],
        format!("{q} -> {r:?}"),
    );
    // LIKE fallback with escapes.
    let like = |term: &str| -> Vec<i64> {
        let sql = format!("SELECT rowid FROM t WHERE {LIKE_ALL} ORDER BY rowid");
        let mut stmt = db.prepare(&sql).unwrap();
        stmt.query_map([like_pattern(term)], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let r = like("发票");
    check("LIKE 2-char zh", r == [14], format!("{r:?}"));
    let r = like("100%");
    check("LIKE escapes %", r == [15], format!("{r:?}"));
    let r = like("a_b");
    check(
        "LIKE escapes _ (no match for a*b)",
        r == [15],
        format!("{r:?}"),
    );
    let r = like("c\\d");
    check("LIKE escapes backslash", r == [15], format!("{r:?}"));
    let r = like("é");
    println!(
        "[INFO] LIKE 'é' -> {r:?}; LIKE 'É' vs 'é' (LIKE folds ASCII only): {:?}",
        like("É")
    );
    let r = like("ab");
    println!("[INFO] LIKE 'ab' (2-char latin) -> {r:?} (rows 2 fullwidth ABC? no; 3 'abc123' yes)");
    let mut s = String::new();
    write!(s, "{:?}", like("IN")).unwrap();
    println!("[INFO] LIKE 'IN' case-insensitive latin -> {s}");
}

#[test]
#[ignore = "F0 probe; run explicitly with --ignored --nocapture"]
fn fts_probe_scale() {
    for n in sizes() {
        for body_percent in [30, 100] {
            let dir = TempDir::new().unwrap();
            println!("\n=== {n} messages, {body_percent}% with body ===");
            let base_db = open(&dir, "base.sqlite3", Variant::None);
            let base = load(
                &base_db,
                &dir,
                "base.sqlite3",
                n,
                body_percent,
                Variant::None,
            );
            println!(
                "  raw text {:.1} MiB; baseline DB (messages+index, no FTS, no text) {:.2} MiB",
                mb(base.raw_bytes as u64),
                mb(base.db_bytes)
            );
            drop(base_db);
            let variants: &[Variant] = if n >= 50_000 && body_percent == 30 {
                &[Variant::Full, Variant::Contentless]
            } else {
                &[Variant::Full]
            };
            for &v in variants {
                let name = format!("{}.sqlite3", v.name());
                let db = open(&dir, &name, v);
                let l = load(&db, &dir, &name, n, body_percent, v);
                let fts_size = l.db_bytes - base.db_bytes.min(l.db_bytes);
                println!(
                    "  [{}] insert {:.1}s ({:.0} rows/s); DB {:.1} MiB; FTS part {:.1} MiB = {:.2}x raw text",
                    v.name(),
                    l.insert.as_secs_f64(),
                    n as f64 / l.insert.as_secs_f64(),
                    mb(l.db_bytes),
                    mb(fts_size),
                    fts_size as f64 / l.raw_bytes as f64
                );
                // Warm the cache, then reopen like a fresh connection.
                bench_queries(&db, v.name(), v == Variant::Full);
                if v == Variant::Full {
                    bench_snippet(&db);
                }
                if v != Variant::Full {
                    // The matches above must still agree with the full table.
                    let c: i64 = db
                        .query_row(
                            "SELECT count(*) FROM fts WHERE fts MATCH ?1",
                            [quote("项目进度")],
                            |r| r.get(0),
                        )
                        .unwrap();
                    println!("  [{}] MATCH(项目进度) count = {c}", v.name());
                }
            }
        }
    }
}
