Query: `object`

```graphql
{
  object(address: "0x1") {
    address
    objects {
      edges {
        node {
          digest
        }
      }
    }
    balance {
      coinType {
        repr
        signature
        layout
        abilities
      }
      coinObjectCount
      totalBalance
    }
    balances {
      edges {
        node {
          coinObjectCount
        }
      }
    }
    coins {
      edges {
        node {
          digest
        }
      }
    }
    stakedIotas {
      edges {
        node {
          digest
        }
      }
    }
    version
    status
    digest
    owner {
      __typename
    }
    previousTransactionBlock {
      digest
      bcs
    }
    storageRebate
    receivedTransactionBlocks {
      edges {
        node {
          digest
          bcs
        }
      }
    }
    bcs
    dynamicField(
      name: {type: "0x0000000000000000000000000000000000000000000000000000000000000001::string::String", bcs: "A2RmMQ=="}
    ) {
      __typename
    }
    dynamicObjectField(
      name: {type: "0x0000000000000000000000000000000000000000000000000000000000000001::string::String", bcs: "A2RmNQ=="}
    ) {
      name {
        bcs
      }
    }
    dynamicFields {
      edges {
        node {
          name {
            __typename
          }
        }
      }
    }
    asMoveObject {
      digest
    }
    asMovePackage {
      digest
    }
  }
}
```

tested by [crates/iota-graphql-e2e-tests/tests/call/owned_objects.move](../../../iota-graphql-e2e-tests/tests/call/owned_objects.move):

```graphql
//# run-graphql
{
  object(address: "0x42") {
    objects {
      edges {
        node {
          owner {
              __typename
              ... on AddressOwner {
              owner {
                  address
              }
            }
          }
        }
      }
    }
  }
}
```

tested by [crates/iota-graphql-e2e-tests/tests/transactions/random.move](../../../iota-graphql-e2e-tests/tests/transactions/random.move):

```graphql
//# run-graphql
{
    object(address: "0x8") {
        address
        version
        asMoveObject {
            contents {
                type { repr }
                json
            }
        }
    }
}
```

tested by [crates/iota-graphql-e2e-tests/tests/call/dynamic_fields.move](../../../iota-graphql-e2e-tests/tests/call/dynamic_fields.move):

```graphql
//# run-graphql
{
  object(address: "@{obj_2_0}") {
    dynamicFields {
      nodes {
        name {
          type {
            repr
          }
          data
          bcs
        }
        value {
          ... on MoveObject {
            __typename
          }
          ... on MoveValue {
            __typename
          }
        }
      }
    }
  }
}
```
