Query: `events`

```graphql
{
  events(first: null, last: null, after: null, before: null, filter: null) {
    edges {
      node {
        __typename
        data
        timestamp
      }
    }
  }
}
```

tested by [crates/iota-graphql-e2e-tests/tests/event_connection/no_filter.move](../../../iota-graphql-e2e-tests/tests/event_connection/no_filter.move):

```graphql
//# run-graphql
{
    events {
        pageInfo {
            hasPreviousPage
            hasNextPage
            startCursor
            endCursor
        }
        nodes {
            json
        }
    }
}

//# run-graphql --cursors {"tx":2,"e":19,"c":1}
{
    events(after: "@{cursor_0}") {
        pageInfo {
            hasPreviousPage
            hasNextPage
            startCursor
            endCursor
        }
        nodes {
            json
        }
    }
}
```
