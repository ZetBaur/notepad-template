# Список вопросов (сведён в один, без повторов)

## Открывашка и small talk
- How are you?
- Small talk: your city / weather

## О себе и карьера
- Tell me about yourself
- Walk me through your resume
- Why are you looking for a new job? / Why are you leaving your current job?
- What are you looking for in your next role?
- Where do you see yourself in 5 years?
- Why so many job changes early in your career?
- What were you doing between [Company A] and [Company B]? / Is there a gap in your CV? (нужен факт владельца — см. TODO)
- What are you most proud of? / Biggest achievement
- What's not on your resume? / What should we know about you?
- Tell me more about [a notable project from your resume]
- Have you applied to our company before, or been in contact with anyone on our team?

## Мотивация (про компанию и роль)
- Why do you want to work here / at our company?
- What do you know about us?
- Why this role / this position?
- Why should we hire you?
- What makes you stand out? / What do you bring that other candidates don't?
- How would you start? What would you do in your first 30/60/90 days?
- Which of our company values resonates with you? / Tell me about a time you showed [our value] (values-based, Amazon/Netflix-стиль)
- What motivates you?

## Условия, формат, документы
- Salary expectations / hourly or yearly rate
- Can you be flexible on compensation?
- When can you start? What is your exact contractual notice period?
- Are you looking for B2B contract work or a permanent employment contract (Festanstellung)?
- Do you have work authorization in Germany / the EU? What visa or residence permit do you hold (EU Blue Card, PR)?
- Remote / hybrid / office. Are you willing to relocate, or strictly remote?
- Are you currently in [your country]? Where are you based, how do we contract with you?
- Why are you sometimes in a different timezone?
- Do you take freelance work alongside your job?
- Do you have other interviews or offers? / Who else are you interviewing with? / Are you interviewing elsewhere?
- What is your current salary / rate? (на скрининге; точную цифру не называть)
- How do you think your current manager/employer will react when you resign? Do you have any holidays booked? (нужен факт владельца — см. TODO)
- Work-life balance
- Are you willing to go the extra mile / work evenings and weekends if needed? (ловушка «мы как семья»)
- How do you feel about a very fast-paced environment where priorities change every day?
- How do you like to work / be managed?
- Team vs. independent work

## Сильные и слабые стороны
- What are your strengths?
- What is your biggest weakness?
- How do others describe you? (coworkers, manager)
- What would your manager say about you?
- What would your manager say you need to improve? (вопрос через третье лицо про зону роста; не только похвала)
- Hobbies / What do you do outside of work? / What do you do in your free time to stay ahead?
- Ownership: how would you teach it to others?

## Конфликты и люди
- Tell me about a conflict / disagreement with a colleague
- Disagreement with your manager / tech lead
- How do you handle disagreements about architecture in your team?
- A difficult coworker / stakeholder
- A difficult manager who micromanaged you
- How do you handle criticism / feedback?
- What do you do when the team picks a decision you disagree with? (disagree and commit)
- A time you gave hard feedback
- How do you work with people who are different from you?
- A time you helped or mentored someone
- How do you onboard a new senior developer into a complex codebase, and share knowledge across the team?
- How do you deal with a colleague who consistently writes poor code or ignores team coding standards in PR reviews?
- What would you do if a colleague performs worse than you / is underperforming? Would you report a colleague to management?

## Ошибки и провалы
- Biggest failure / mistake
- Tell me about a time you were wrong about something. How did you handle it?
- A time you missed a deadline
- A production incident / a bug you found in production
- How do you conduct post-mortems after a major outage, and make sure it never happens again?
- A decision you regret / what would you do differently?
- A time you didn't know something (new domain)

## Давление, сроки, приоритеты
- How do you handle pressure / tight deadlines? (A time you worked under a tight deadline)
- How do you prioritize when everything is urgent?
- Multiple projects at once
- Working with unclear requirements / ambiguity
- A time plans changed quickly / priorities shifted mid-sprint and your work had to be scrapped or refactored
- A time you had to compromise on your ideal technical solution due to business or time constraints

## Лидерство, инициатива, бизнес
- Have you led a team?
- A time you led a project / took ownership
- A time you identified a gap outside your responsibilities and took full ownership to solve it
- A time you took initiative
- A time you improved a process
- A time you influenced a decision without authority
- A time you made an unpopular decision / persuaded the team
- Biggest technical achievement / a project you are technically proud of
- How do you evaluate the business impact of your technical decisions? An example where a technical optimization improved a business metric?
- How do you convince non-technical stakeholders (Product, Executives) to allocate time for technical debt or refactoring?
- How do you collaborate with Product, Design, and QA to turn business goals into technical requirements?
- Have you worked on cloud cost optimization (AWS)? How did you reduce infrastructure expenses?

## Технические поведенческие
- A hard technical problem you solved
- A time you improved the performance of an application
- How do you make technical decisions / choose between options?
- How do you keep learning / stay up to date?
- A time you learned a new technology quickly
- How do you approach code review? / How do you handle review comments?
- Quality vs speed — how do you balance?
- Is it better to deliver perfect and late, or good and on time?
- A manager asks for a "small feature" in the middle of the sprint. How do you respond? / How do you say no to a manager or stakeholder? (scope creep)
- Legacy code / tech debt
- Explaining technical things to non-technical people
- Favorite programming language / tools
- Do you use AI tools at work?
- How would you describe your experience with Node.js?
- What is your experience with React Native?
- Experience with Vue / MongoDB / AWS / Angular
- How much of your work is mobile vs web vs backend?
- How involved were you with payment features?
- Side projects / your desktop app / are those articles yours?

## Технические вопросы: Node.js и JavaScript
- What is the difference between let and const in JavaScript?
- Explain how closures work in JavaScript.
- What is the difference between Promise.all and Promise.allSettled?
- How does the event loop work in Node.js?
- How do microtasks (Promises, `process.nextTick`) differ from macrotasks (`setTimeout`, `setImmediate`) in execution order?
- How do you find and fix memory leaks or heap out-of-memory errors in production Node.js?
- When does Node.js use the `libuv` thread pool, and when would you adjust `UV_THREADPOOL_SIZE`?
- How do you handle CPU-intensive tasks without blocking the Event Loop (Worker Threads vs Streams vs Clustering)?
- What is the difference between useMemo and useCallback in React?

## Технические вопросы: базы данных и кэш
- How do you find and fix a slow PostgreSQL query?
- What are database isolation levels, and how do you prevent race conditions or phantom reads under high concurrency?
- How do you handle cache invalidation, cache stampede and cache penetration with Redis?
- When would you choose sharding over read-replicas, and how do you choose a shard key for PostgreSQL?

## Технические вопросы: API, очереди, микросервисы
- What is the difference between REST and GraphQL? How do you decide between them on a project?
- How would you design a rate limiter for an API?
- How do you handle eventual consistency and distributed transactions across microservices (Saga vs Two-Phase Commit)?
- How do you ensure "at-least-once" or "exactly-once" processing in Kafka or RabbitMQ, and implement idempotency in consumers?
- How do you implement backpressure when consuming streams or large message volumes from queues?
- How do you handle API versioning and database schema migrations without downtime or breaking clients?
- How do you secure microservices internally (mTLS, API Gateways) and externally (OAuth2, JWT revocation)?

## System Design
- How would you design a URL shortener, a Notification System, or a Real-Time Ride-Hailing Backend to handle 50,000+ RPS?
- How do you set up distributed tracing, structured logging, and metrics (Prometheus, Grafana, Datadog) with meaningful alerting (SLI/SLO/SLA)?

## Необычные и каверзные (vibe-вопросы, профайлинг)
- On a scale of 1 to 10, how lucky are you?
- Sell me this pen / this glass of water
- If you could have any superpower for work, what would it be?
- If you won a million dollars tomorrow, would you still show up for work?
- You're stuck on a desert island. You can only have one thing. What is it?
- When was the last time you realized you were a bad person? (провокация; не поддаваться)
- If you were an animal, which one would you be? / What really makes you tick? / I'm sensing some hesitation here...

## Личные и некорректные вопросы (не должны задаваться)
- Are you married? Do you have children? Who looks after your kids when you travel for work?
- How old are you? / When did you graduate high school?
- Where are you originally from? / Is English your native language?
- Do you observe any religious holidays?

## Вопросы к ним и закрытие
- Do you have any questions for us?
- What does the deployment pipeline look like, and how long does a commit take to reach production?
- What percentage of team capacity goes to refactoring, tech debt and infrastructure vs new features?
- How are engineering teams structured, and who has the final say on architectural decisions?
- What is the biggest technical or scaling bottleneck the team is trying to solve right now?
- What would a successful first 3 months look like for someone in this role?
- How does the team handle technical debt or unexpected roadblocks?
- What do you personally like most about working here?
- How does the team handle urgent tasks that arise outside standard working hours? (проверка культуры)
- Can you describe how a task moves from an idea to production? How often do priorities change mid-sprint?
- How does the company support long-term professional growth and prevent burnout?
- Is there an internal candidate being considered for this role as well? (рекрутёру, на первом звонке)
- Closing statement (короткое резюме в конце интервью)
