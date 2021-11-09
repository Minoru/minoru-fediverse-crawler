# Architecture

## The purpose of this project

minoru-fediverse-crawler goes between Fediverse servers, fetches their peers
lists, and compiles a summary list of alive known instances.

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
    but they don't have crawlers. This project should fill that niche without
    duplicating the work of others.
- **horizontal scaling** (mesh or peer-to-peer architecture). This *could*
    increase reliability and availability, but it would also increase
    complexity. Even though Fediverse instances show up and disappear every day,
    it doesn't happen so fast that the crawler needs five-nines SLA, and
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
2. the crawler downloads data from Fediverse instances.

We block the first vector by making the front-end static: the service
periodically updates a static JSON file which is then served from the filesystem
by Nginx. (We still have to secure Nginx, but that's a better-known problem than
securing a custom frontend.)

The second vector still allows for a number of attacks, which are described
below.

### Overview of attacks

There are three classes of possible attacks:

1. attacks against the network stack (e.g. exploiting bugs in the TLS
   implementation);
2. attacks against the code that processes the responses (e.g. exhausting the
   crawler's memory using a specifically crafted JSON response);
3. attacks against the inner workings of the crawler (e.g. feeding it seemingly
   valid data that makes the crawler send a lot of requests to a single server,
   causing a denial of service of that server).

The first two are mostly mitigated by:

1. using a memory-safe language;
2. sandboxing untrusted parts of the crawler.

   The main part of the crawler, let's call it Orchestrator, is trusted. Every
   time it wants to check some Fediverse instance, the Orchestrator spawns a new
   Checker process. Checker can communicate data back to the Orchestrator via
   a Unix pipe connected to its stdout.

   Checkers are untrusted, and only have access to CPU, memory, and network —
   they can't write into the database directly.

   Thus, compromising a Checker gives an attacker only a small foothold, and
   from there they have to fight through the narrow pipe of IPC. Attacks like
   memory exhaustion also have limited effect because they likely crash just
   that one process.

The rest of the mitigations are smaller, more focused measures; they are
detailed below.

### Specific attacks

#### Attacks against the crawler itself

This set of attacks tries to slow down, incapacitate, or mislead the crawler.

##### Feed the crawler bogus data to make the front-end advertise it

The front-end lists currently alive instances. Thus, by creating a fake
instance, an attacker could make the service advertise it. The only goal we can
think of is spam.

**There is no mitigation** at the moment, but there are ideas. One we have
stolen from fediverse.space is to group instances by the registrable part of the
domain ("example.com" in "foo.example.com"), and require manual moderation of
large groups. See https://github.com/Minoru/minoru-fediverse-crawler/issues/19
for details and discussion.

##### Slowing the crawler down

This could be accomplished in a variety of ways: large responses, slow
responses, low transfer speeds, redirect loops.

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

##### Crashing the crawler

This could be accomplished by sending particularly broken responses to the
crawler.

Mitigations:

1. the part of the crawler that processes the response should be a separate
   process, which can crash without affecting the rest of the service. (It can't
   be a thread or a simple `try-catch` block since those are not fully isolated
   and could e.g. corrupt memory).

#### Making the crawler attack the Internet

This set of attacks uses the crawler to cause or amplify attacks against hosts on
the Internet.

##### Pointing the crawler at a host to harass it

The crawler will visit any host it is told about; the attacker could use it as
a help in a DoS attack.

Mitigations:

1. make checks so rarely that the crawler is useless as part of a DoS attack.
   Unfortunately, this puts a limit on how often the crawler can check instances;
   e.g. at 1 check per second it would take almost 12 days to crawl 1 million
   instances. I still think it'd be worth it, since the Fediverse is likely to
   expand slower as it gets larger;
2. if a host doesn't identify as a Fediverse instance for a while, declare it
   "dead" and re-check it way less often.

##### Redirecting the crawler to a victim host

The previous attack can still be mounted if an attacker could somehow
concentrate the crawler's requests at the victim host. There are two conceivable
ways to do that: make a lot of DNS CNAMEs, or set up hosts that serve HTTP
redirects to the victim.

Thus, we follow redirects only as long as they point to the exact same origin
(schema, domain, port triple).

Mitigations:

1. compare URLs from the NodeInfo with the hostname by which the NodeInfo was
   fetched. If they don't match, consider this host "dead". Thus we never touch
   the "victim" host, and no attack is possible with DNS CNAMEs;
2. when encountering a temporary redirect, don't follow it, and mark the checked
   instance as "dead". If the instance isn't malicious, we'll learn about its
   new hostname soon enough from some other server. If it's malicious, we just
   stopped an attack;
3. when encountering a permanent redirect, add the target to the database, and
   mark the checked instance as "moved". This lets us "coalesce" multiple
   redirects into one (because the database only stores each instance once).

## Architecture

The service is a single executable. All the data is stored in SQLite. These
choices make it easy to deploy and maintain the service.

The code is written in Rust. It's what I know fairly well, and is a good fit for
a backend service like this.

The main process is called an Orchestrator. It looks through the list of known
Fediverse instances, finds the ones that are due to get checked, and spawns OS
threads that start Checker processes for each check. It also immediately
reschedules these checks for later. From time to time it also generates a dump
of all known alive instances, which is then served by Nginx to the general
public.

Each check is performed by a separate Checker process. It communicates with the
Orchestrator via a Unix pipe. This process acts as a sandbox, enhancing
security. (Once we got an MVP, it's our intent to strengthen the sandbox with
chroot, namespaces, and seccomp.)

First of all, the Checker process fetches robots.txt against which all other
requests will be checked.

Next, it fetches NodeInfo of the instance. If that succeeds, and the response is
a valid NodeInfo document, the instance is considered to be alive (which is
immediately reported back to the Orchestrator).

Then, the checker looks at the `software.name` field of the NodeInfo and can
make an additional request to check if an instance is "private", i.e. if it
opted out of statistics. Only GNU Social, Hubzilla and Friendica support this at
the moment.

After that, the Checker picks an appropriate API endpoint to request the list of
peers; if the software name is unknown, no further requests are made. The
response is parsed and the list of peers is reported to the Orchestrator.

A thread that the Orchestrator starts for each check is responsible for reading
Checker's responses and storing them in the database. If the Checker never
writes anything before terminating, the instance is considered dead. If the
Checker says that the instance is moving (temporary redirect), then it's marked
dead; if it has moved (permanent redirect), then it is marked as moved. As new
instances are found in the peer list, they're assigned a random time to get
checked in the near future.

## Discussion of the architecture

### Performance considerations

All checks would require at least a single write: the Orchestrator has to update
the next check's datetime. "Moving" and "dying" instances will cause an
additional write to update the count of redirects and failed checks. If a new
peer is found in someone's peer list, that would cause yet another write.

Thus, the database is at the center of everything, and SQLite only supports one
writer at a time. We might have to work on our SQL queries to reduce the number
of writes, but we believe that the problem is not dire enough to switch to
another database.

### Async Rust vs. OS threads

We use async Rust in Checkers because that's the easy way to integrate with the
reqwest crate. We do not use async in the Orchestrator because all its
operations have to talk to the database, and SQLite operations are blocking;
we'd have to spawn separate OS threads for them anyway, and there is no point
hiding this behind async.
