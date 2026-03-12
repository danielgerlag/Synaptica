/// MCP prompt templates for graph database interactions.

pub struct PromptTemplate {
    pub name: &'static str,
    pub description: &'static str,
    pub content: &'static str,
}

pub fn all_prompts() -> Vec<PromptTemplate> {
    vec![
        PromptTemplate {
            name: "gql_query_help",
            description: "GQL syntax reference for Synaptica — explains all supported query operations with examples.",
            content: GQL_QUERY_HELP,
        },
        PromptTemplate {
            name: "graph_exploration",
            description: "Step-by-step template for exploring an unknown graph database.",
            content: GRAPH_EXPLORATION,
        },
        PromptTemplate {
            name: "data_modeling",
            description: "Template for designing and creating a graph data model from requirements.",
            content: DATA_MODELING,
        },
    ]
}

const GQL_QUERY_HELP: &str = r#"# GQL Query Reference for Synaptica

Synaptica implements the GQL standard (ISO/IEC 39075:2024). Here is a concise reference:

## Reading Data

```gql
-- Match all nodes with a label
MATCH (n:Person) RETURN n.name, n.age

-- Match with filters
MATCH (n:Person) WHERE n.age > 30 RETURN n.name, n.age ORDER BY n.age DESC

-- Match relationships
MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN a.name, b.name, r.since

-- Pagination
MATCH (n:Person) RETURN n.name LIMIT 10 OFFSET 20

-- Aggregation
MATCH (n:Person) RETURN n.city, COUNT(*) AS count GROUP BY n.city

-- Distinct values
MATCH (n:Person) RETURN DISTINCT n.city
```

## Creating Data

```gql
-- Create a node
INSERT (:Person {name: 'Alice', age: 30})

-- Create an edge (MUST use MATCH to find endpoints first)
MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'})
INSERT (a)-[:KNOWS {since: 2024}]->(b)
```

## Updating Data

```gql
-- Set a property
MATCH (n:Person {name: 'Alice'}) SET n.age = 31
```

## Deleting Data

```gql
-- Delete a node (must have no edges)
MATCH (n:Person {name: 'Alice'}) DELETE n

-- Delete a node and all its edges
MATCH (n:Person {name: 'Alice'}) DETACH DELETE n
```

## Indexing

```gql
-- Create an index for faster lookups
CREATE INDEX ON :Person(name)

-- Drop an index
DROP INDEX ON :Person(name)
```

## Supported Functions

- Aggregates: `COUNT(*)`, `SUM(expr)`, `AVG(expr)`, `MIN(expr)`, `MAX(expr)`, `COLLECT(expr)`
- String: `toString()`, `size()`
- Type: `toInteger()`, `toFloat()`, `labels(n)`, `type(r)`, `id(n)`, `keys(n)`

## Important Notes

- Edge INSERT always requires a preceding MATCH clause to identify the source and target nodes.
- Property values can be strings, integers, floats, booleans, lists, or maps.
- Use single quotes for string literals: `'Alice'`, not `"Alice"`.
"#;

const GRAPH_EXPLORATION: &str = r#"# Graph Exploration Template

Follow these steps to explore an unknown Synaptica graph database:

## Step 1: Check the schema
Use the `get_schema` tool to see all node/edge labels and their properties.

## Step 2: Sample some data
For each node label discovered:
```gql
MATCH (n:LabelName) RETURN n LIMIT 5
```

## Step 3: Explore relationships
```gql
MATCH (a)-[r]->(b) RETURN labels(a), type(r), labels(b), COUNT(*) GROUP BY labels(a), type(r), labels(b)
```

## Step 4: Check cardinality
```gql
MATCH (n:LabelName) RETURN COUNT(*) AS total
```

## Step 5: Examine indexes
Use the `list_indexes` tool to see what's indexed for fast lookups.

## Step 6: Run targeted queries
Based on what you've learned, write specific queries to answer your questions.
"#;

const DATA_MODELING: &str = r#"# Data Modeling Template

Use this template to design and create a graph data model in Synaptica.

## Step 1: Identify entities (node labels)
List the main entities in your domain. Each becomes a node label.
Example: Person, Company, Product, Order

## Step 2: Identify relationships (edge labels)
Determine how entities relate. Each becomes an edge label.
Example: WORKS_AT (Person→Company), PURCHASED (Person→Product)

## Step 3: Define properties
For each entity and relationship, list the properties.
Example: Person {name, email, age}, WORKS_AT {since, role}

## Step 4: Create the data

```gql
-- Create nodes
INSERT (:Person {name: 'Alice', email: 'alice@example.com', age: 30})
INSERT (:Company {name: 'Acme Corp', founded: 2010})

-- Create relationships
MATCH (p:Person {name: 'Alice'}), (c:Company {name: 'Acme Corp'})
INSERT (p)-[:WORKS_AT {since: 2020, role: 'Engineer'}]->(c)
```

## Step 5: Add indexes for frequently queried properties
```gql
CREATE INDEX ON :Person(name)
CREATE INDEX ON :Person(email)
CREATE INDEX ON :Company(name)
```

## Step 6: Verify
```gql
MATCH (p:Person)-[r:WORKS_AT]->(c:Company)
RETURN p.name, r.role, c.name
```
"#;
