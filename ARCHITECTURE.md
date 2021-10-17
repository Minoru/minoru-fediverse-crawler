# Architecture

## The purpose of this project

fediverse.observer goes around the Fediverse, finds recently created instances,
and reports them via a webpage and an Atom feed. It also provides a plain text
list of all currently known running instances.

## The goals of the architecture

The service should be:

- easy to set up
- easy to run and maintain
- prepared to crawl one million nodes
- reasonably secure

Some explicit *non*-goals are:

- **providing a wealth of information**: uptime statistics, software make and
    version etc. There are already [sites that do
    this](https://git.feneas.org/feneas/fediverse/-/wikis/instance-monitoring-sites),
    but they don't have spiders. Fediverse.observer should fill that niche
    without duplicating the work of others.
- **horizontal scaling** (mesh or peer-to-peer architecture). This *could*
    increase reliability and availability, but it would also increase
    complexity. Even though Fediverse instances show up and disappear every day,
    it doesn't happen so fast that Fediverse.observer needs five-nines SLA, and
    neither do its users. I believe that one million nodes can be crawled using
    a single server, so I don't think horizontal scaling is necessary here.
- **portability**: there is only going to be a single instance of this service,
    so it doesn't make much sense to make the software extremely portable. We'll
    be targeting up-to-date Linux, because that's what I know best.

## Security

All feedback on this section is welcome by email at eual.jp@gmail.com.
Encrypting it to PGP key 0x356961a20c8bfd03 would be a nice touch.

### Attack vectors

The service is connected to the world in two ways:

1. anyone on the Internet can visit the service's web frontend; and
2. the spider downloads data from Fediverse instances.

We block the first vector by making the front-end static: the service
periodically updates static HTML, Atom and CSV files which are then served from
the filesystem by Nginx. (We still have to secure Nginx, but that's a better-known
problem than securing a custom frontend.)

The second vector still allows for a number of attacks, which are described
below.

### Overview of attacks

There are three classes of possible attacks:

1. attacks against the network stack (e.g. exploiting bugs in the TLS
   implementation);
2. attacks against the code that processes the responses (e.g. exhausting the
   spider's memory using a specifically crafted JSON response);
3. attacks against the inner workings of the spider (e.g. feeding it a seemingly
   valid data that makes the spider send a lot of requests to a single server,
   causing a denial of service)

The first two are mitigated by:

1. using a memory-safe language;
2. sandboxing untrusted parts of the spider.

   The main part of the spider, let's call it "orchestrator", is trusted. Every
   time it wants to check some Fediverse instance, the orchestrator spawns a new
   "checker" process and the associated "network" process; both of those are
   untrusted. "Checker" can communicate data back to the orchestrator, and it
   also has two-way communications with its associated "network" process. Each
   receiver validates what it gets.

   The "checker" and the "network" processes are untrusted and thus sandboxed.
   The "checker" only has access to CPU and memory; the "network" process has
   access to CPU, memory, and network.

   Thus, compromising any one process gives an attacker a small foothold, and
   from there they have to fight the validators through the narrow pipe of IPC.
   Attacks like memory exhaustion also have limited effect because they likely 
   crash just that one process.

The last attack class is mitigated through smaller, more focused measures which
are detailed below.

### Specific attacks

These are the attacks from the third class (see above).

#### Attacks against the spider itself

This set of attacks tries to slow down, incapacitate, or mislead the spider.

##### Feed the spider bogus data to make the front-end advertise it

The front-end lists currently alive instances. Thus, by creating a fake
instance, an attacker could make the service advertise it. The goals could be:

1. black SEO: a link from the service boosts the search rating of the target
2. spam: filling the service with bogus data

Mitigations:

1. annotate all links with `rel="nofollow"`, so they don't boost the target;
2. **there is no mitigation for the spam problem**.

##### Slowing the spider down

This could be accomplished in a variety of ways: slow responses, low transfer
speeds, redirect loops.

Mitigations:

1. talk to Fediverse instances concurrently, so a slow instance doesn't affect
   working with others;
2. put timeouts on network operations;
3. put limits on the number of redirects;
4. set a time limit on the entire check (so even if an attacker manages to evade
   the aforementioned mitigations, or if they were misconfigured, the service is
   still protected).

##### Resource exhaustion

An attacker could try to exhaust memory, disk space, or hog the CPU. This could
be accomplished by sending large responses, complex responses, or by varying the
details that are stored to disk.

Mitigations:

1. do not process responses larger than a certain threshold;
2. use incremental algorithms to keep memory use in check;
3. the database should store as little information as possible, making it hard
   to exhaust disk space.

##### Crashing the spider

This could be accomplished by sending particularly broken responses to the
spider.

Mitigations:

1. the part of the spider that processes the response should be a separate
   process, which can crash without affecting the rest of the service. (It can't
   be a thread or a simple `try-catch` block since those are not fully isolated
   and could e.g. corrupt memory).

#### Making the spider attack the Internet

This set of attacks uses the spider to cause or amplify attacks against hosts on
the Internet.

##### Pointing the spider at a host to harass it

The spider will visit any host it is told about; the attacker could use it as
a help in a DoS attack.

Mitigations:

1. make checks so rarely that the spider is useless as part of a DoS attack.
   Unfortunately, this puts a limit on how often the spider can check instances;
   e.g. at 1 check per second it would take almost 12 days to crawl 1 million
   instances. I still think it'd be worth it, since the Fediverse is likely to
   expand slower as it gets larger;
2. if a host doesn't identify as a Fediverse instance for a while, declare it
   "dead" and re-check it way less often.

##### Redirecting the spider to a victim host

The previous attack can still be mounted if an attacker could somehow
concentrate the spider's requests at the victim host. There are two conceivable
ways to do that: make a lot of DNS CNAMEs, or set up hosts that serve HTTP
redirects to the victim.

Thus, we follow redirects as long as they point to the same host, and stop when
we encounter a new hostname.

Mitigations:

1. compare URLs from the NodeInfo with the hostname by which the NodeInfo was
   fetched. If they don't match, consider this host "dead". This means further
   checks will be made less often, making the DNS CNAMEs useless;
2. when encountering HTTP 302 Moved Permanently (which points to a new
   hostname), add the target to the list of known instances, and check the old
   hostname as "dead";
3. when encountering HTTP 302 Found (which points to a new hostname), stop and
   re-schedule the check. Either the redirect will disappear after a while, or
   the server will move for good and the service will re-discover it.

## Architecture

The service is a single executable. All the data is stored in SQLite or sled
(haven't decided yet). These choices make it easy to deploy and maintain the
service.

The code is written in Rust. It's what I know fairly well, and is a good fit for
a backend service like this.

The main process is called an "orchestrator". It looks through the list of known
Fediverse instances, finds the ones that are due to get checked, and starts
processes for each check. From time to time it also generates a dump of all
known alive instances, which is then served by Nginx to the general public.

Each check is performed by a separate "checker" process. It communicates with
the "orchestrator" via a Unix pipe. This process acts as a sandbox, enhancing
security. (Once we got an MVP, it's our intent to strengthen the sandbox with
chroot, namespaces, and seccomp.)

First of all, the "checker" process fetches NodeInfo of the instance. If that
succeeds, and the response is a valid NodeInfo document, the instance is
considered to be alive (which is immediately reported back to the
"orchestrator").

After that, the "checker" looks at the `software.name` field of the NodeInfo and
picks an appropriate API endpoint to request the list of peers; if the software
name is unknown, Mastodon's endpoint (api/v1/instance/peers) is used, as it is
the most common. The response is parsed and is reported to the "orchestrator".

Upon spawning a "checker" process, the "orchestrator" starts a one-minute timer.
If the timer fires before the process has finished, the process is forcibly
killed.

When a "checker" process reports that an instance is alive, the "orchestrator"
picks the datetime of the next check and records it in the database. When
a "checker" process starts reporting peer instances, the "orchestrator" checks
if they are already in the database, and if they aren't, adds them along with
a random datetime of the next check.

## Discussion of the architecture

### Performance considerations

It's bad that the "orchestrator" is the one who checks if the instance is
already in the database. New instances appear quite rarely, so most of the time,
lists of peers will contain nothing new. At the same time, the lists will only
grow over time as Fediverse itself grows. As a result, the "orchestrator" will
find itself processing vast amounts of data for little gain.
Back-of-the-envelope math: 1 million (1e6) instances, each talking to half the
Fediverse (5e5 instances), means there are 5e11 entries in all lists combined;
spread over one day, this is 5e11/24/60/60 = 5.8e6 entries per second to
process.

Due to security considerations, we can't give the "checker" process access to
the database. Instead, we could give it a list of known instances, so each
process only report new ones. Details are lacking at this point; the important
bit is that this architecture *can* be scaled to 1e6 instances.

## Various notes

Could use this for http mocking and testing: https://github.com/alexliesenfeld/httpmock

This video an async-await might come in handy for "orchestrator": https://fosstodon.org/@jonhoo/106871799020652794

How to make this reliable enough for production: https://pythonspeed.com/fil/docs/fil4prod/reliable.html

Peer lists in different Fedi software:
- mastodon, pleroma, misskey, bookwyrm: api/v1/instance/peers
- peertube: server/following https://docs.joinpeertube.org/api-rest-reference.html#tag/Instance-Follows/paths/~1server~1followers~1{nameWithHost}~1accept/post

### Sandboxing

https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/design/sandbox.md
https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/linux/sandboxing.md
https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/linux/suid_sandbox.md
https://www.usenix.org/conference/enigma2021/presentation/palmer
https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/design/sandbox_faq.md
https://wiki.mozilla.org/Security/Sandbox
https://wiki.mozilla.org/Security/Sandbox/Specifics#Linux
https://lwn.net/Articles/531114/
https://github.com/kristapsdz/acme-client-portable/blob/master/Linux-seccomp.md

## Tentative SQL schema

instances
- id integer unique not null primary key
- hostname string unique not null
- state referrences states(id) not null

states
- id integer unique not null primary key
- state text not null
(This table contains: "discovered", "live", "dying", "dead", "reviving",
"moving", "moved")

state_live
- id integer unique not null primary key
- live_since datetime not null

state_dying
- id integer unique not null primary key
- dying_since datetime not null

state_dead
- id integer unique not null primary key
- dead_since datetime not null

state_reviving
- id integer unique not null primary key
- reviving_since datetime not null

state_moving
- id integer unique not null primary key
- moving_since datetime not null
- moving_to referencing instances(id)

state_moved
- id integer unique not null primary key
- moved_at datetime not null
- moved_to referencing instances(id)

schedule
- instance referencing instances(id)
- next_check_after datetime not null
