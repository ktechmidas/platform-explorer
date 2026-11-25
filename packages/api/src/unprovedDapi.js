// Raw bytes straight from DAPI, without a proof, for objects whose wire format the SDK's DPP
// cannot read. What it cannot decode it cannot verify either, so the two raw routes fall back to
// this and mark their answer as unverified.
const { IdentifierWASM } = require('pshenmic-dpp')
const { encode } = require('cbor-x')

let proto

const loadProto = async () => {
  proto ??= await import('dash-platform-sdk/proto/generated/platform.js')

  return proto
}

const resultOf = (response, kind) => {
  const v0 = response.version?.oneofKind === 'v0' ? response.version.v0 : null

  return v0?.result?.oneofKind === kind ? v0.result[kind] : null
}

const getUnprovedDataContract = async (sdk, identifier) => {
  const { GetDataContractRequest } = await loadProto()

  const request = GetDataContractRequest.create({
    version: { oneofKind: 'v0', v0: { id: new IdentifierWASM(identifier).bytes(), prove: false } }
  })

  const { response } = await sdk.dataContracts.grpcPool.getClient().getDataContract(request)
  const dataContract = resultOf(response, 'dataContract')

  return dataContract?.length ? Buffer.from(dataContract) : null
}

const getUnprovedDocument = async (sdk, dataContractId, documentTypeName, identifier) => {
  const { GetDocumentsRequest } = await loadProto()

  const request = GetDocumentsRequest.create({
    version: {
      oneofKind: 'v0',
      v0: {
        dataContractId: new IdentifierWASM(dataContractId).bytes(),
        documentType: documentTypeName,
        where: encode([['$id', '=', new IdentifierWASM(identifier).base58()]]),
        orderBy: encode([]),
        limit: 1,
        prove: false
      }
    }
  })

  const { response } = await sdk.documents.grpcPool.getClient().getDocuments(request)
  const [document] = resultOf(response, 'documents')?.documents ?? []

  return document?.length ? Buffer.from(document) : null
}

module.exports = { getUnprovedDataContract, getUnprovedDocument }
